//! The llama.cpp engine: one GGUF instruct model (Qwen2.5 / Llama-3.2
//! instruct and similar) and one context, owned by a dedicated thread.
//!
//! Owning the model and context on one thread avoids a self-referential
//! model/context struct, and it serialises requests onto the single context
//! for free.
//!
//! Latency comes from three things:
//! * **Prefix reuse.** The KV cache keeps the last prompt. Only the tokens
//!   after the first difference are prefilled. flow-core's `build_prompt`
//!   keeps the ~1k-token system prompt static per level/style, so a warm
//!   call prefills only the user message.
//! * **Warm-up.** `load` prefills a caller-supplied prompt (flow-core sends
//!   its default light/default-style one), so the first dictation is warm.
//! * **Bounded decode.** Greedy sampling, at most 2 × transcript tokens + 32
//!   new tokens (03 §7.5), stop on EOG or a stop string, and a deadline and
//!   cancel flag checked between prefill chunks and before every token.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

/// The system prompt (~1k tokens) + user message (≤ ~250 tokens of
/// context) + output fits comfortably.
const N_CTX: u32 = 4096;
/// Prefill chunk size. It is also the granularity of the cancel check
/// during prefill.
const N_BATCH: usize = 128;

/// Text after which the model is rambling past its answer. EOG tokens
/// cover normal termination; these catch template leaks and the model
/// starting another example or user turn.
const STOP_STRINGS: &[&str] = &[
    "<|im_end|>",
    "<|im_start|>",
    "<|eot_id|>",
    "<|end_of_text|>",
    "<|endoftext|>",
    "</s>",
    "\nTRANSCRIPT:",
    "\nEXAMPLE",
    "\nAPP:",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    /// (role, content) in order: system, example turns, final user message.
    pub messages: Vec<(String, String)>,
    /// The raw transcript alone; sizes the output budget.
    pub transcript: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanErr {
    /// Deadline passed or the caller raised the cancel flag.
    Cancelled,
    Backend(String),
}

/// What the loaded model reports.
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub n_params: u64,
    pub chat_template: bool,
    pub warm_prefill_tokens: usize,
    pub warm_ms: u64,
    pub load_ms: u64,
}

/// Counters of the most recent `clean` call (benchmarks, the CLI).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub prompt_tokens: usize,
    /// Tokens actually decoded in prefill (prompt minus the reused prefix).
    pub prefill_tokens: usize,
    pub prefill_ms: u64,
    pub gen_tokens: usize,
    pub gen_ms: u64,
    /// Forward passes during generation (< gen_tokens when drafts hit).
    pub gen_steps: usize,
    pub draft_accepted: usize,
}

/// llama.cpp may be initialised once per process.
static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, String> {
    BACKEND
        .get_or_init(|| {
            let mut b = LlamaBackend::init().map_err(|e| e.to_string())?;
            if std::env::var_os("FLOW_LLAMA_LOG").is_none() {
                b.void_logs();
            }
            Ok(b)
        })
        .as_ref()
        .map_err(Clone::clone)
}

struct Job {
    prompt: Prompt,
    deadline: Instant,
    cancel: *const AtomicBool,
    reply: mpsc::Sender<Result<String, CleanErr>>,
}

// SAFETY: `cancel` is either null or points at an AtomicBool that the
// caller of `Engine::clean` keeps alive until the reply arrives, and
// `clean` blocks on that reply.
unsafe impl Send for Job {}

pub struct Engine {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    stats: Arc<Mutex<Stats>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    pub info: Info,
}

impl Engine {
    /// Load `path` (GGUF), create the context and prefill `warm`. Blocks
    /// until the model is ready.
    pub fn load(path: &str, threads: usize, warm: Option<Prompt>) -> Result<Self, String> {
        if !Path::new(path).is_file() {
            return Err(format!("llm model not found: {path}"));
        }
        let t0 = Instant::now();
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<Info, String>>();
        let p = path.to_string();
        let threads = threads.max(1) as i32;
        let stats = Arc::new(Mutex::new(Stats::default()));
        let wstats = stats.clone();
        let worker = std::thread::Builder::new()
            .name("flow-llm".into())
            .spawn(move || {
                let setup = || -> Result<(LlamaModel, LlamaContextParams), String> {
                    let model =
                        LlamaModel::load_from_file(backend()?, &p, &LlamaModelParams::default())
                            .map_err(|e| format!("llm load: {e}"))?;
                    let cparams = LlamaContextParams::default()
                        .with_n_ctx(NonZeroU32::new(N_CTX))
                        .with_n_batch(N_BATCH as u32)
                        .with_n_ubatch(N_BATCH as u32)
                        .with_n_threads(threads)
                        .with_n_threads_batch(threads);
                    Ok((model, cparams))
                };
                let (model, cparams) = match setup() {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let ctx = match backend().and_then(|b| {
                    model
                        .new_context(b, cparams)
                        .map_err(|e| format!("llm context: {e}"))
                }) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut st = State {
                    template: model.chat_template(None).ok(),
                    model: &model,
                    ctx,
                    cached: Vec::new(),
                    stats: Stats::default(),
                };
                let tw = Instant::now();
                let warm_prefill_tokens = warm.map_or(0, |w| st.warm(&w));
                let info = Info {
                    n_params: model.n_params(),
                    chat_template: st.template.is_some(),
                    warm_prefill_tokens,
                    warm_ms: tw.elapsed().as_millis() as u64,
                    load_ms: 0,
                };
                if ready_tx.send(Ok(info)).is_err() {
                    return;
                }
                for job in jobs_rx {
                    // SAFETY: see `unsafe impl Send for Job`.
                    let cancel = unsafe { job.cancel.as_ref() };
                    st.stats = Stats::default();
                    let r = st.run(&job.prompt, job.deadline, cancel);
                    *wstats.lock().unwrap() = st.stats;
                    let _ = job.reply.send(r);
                }
            })
            .map_err(|e| format!("spawn llm thread: {e}"))?;
        let mut info = match ready_rx.recv() {
            Ok(Ok(i)) => i,
            Ok(Err(e)) => {
                let _ = worker.join();
                return Err(e);
            }
            Err(_) => {
                let _ = worker.join();
                return Err("llm thread died during load".into());
            }
        };
        info.load_ms = t0.elapsed().as_millis() as u64;
        Ok(Self {
            jobs: Mutex::new(Some(jobs_tx)),
            stats,
            worker: Mutex::new(Some(worker)),
            info,
        })
    }

    /// Clean one prompt. Blocks until done, the deadline passes or `cancel`
    /// is raised (then `Err(Cancelled)` within about one decode step).
    pub fn clean(
        &self,
        prompt: &Prompt,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, CleanErr> {
        let (tx, rx) = mpsc::channel();
        {
            let jobs = self.jobs.lock().unwrap();
            let Some(jobs) = jobs.as_ref() else {
                return Err(CleanErr::Backend("llm unloaded".into()));
            };
            jobs.send(Job {
                prompt: prompt.clone(),
                deadline,
                cancel: cancel.map_or(std::ptr::null(), |c| c as *const AtomicBool),
                reply: tx,
            })
            .map_err(|_| CleanErr::Backend("llm thread gone".into()))?;
        }
        rx.recv()
            .map_err(|_| CleanErr::Backend("llm thread gone".into()))?
    }
}

impl Engine {
    /// Counters of the last finished `clean` call.
    pub fn last_stats(&self) -> Stats {
        *self.stats.lock().unwrap()
    }
}

impl Drop for Engine {
    /// Unload: closing the job channel ends the thread, which frees the
    /// context and then the model.
    fn drop(&mut self) {
        self.jobs.lock().unwrap().take();
        if let Some(w) = self.worker.lock().unwrap().take() {
            let _ = w.join();
        }
    }
}

struct State<'m> {
    model: &'m LlamaModel,
    template: Option<LlamaChatTemplate>,
    ctx: LlamaContext<'m>,
    /// Tokens whose KV entries are in sequence 0, positions 0..len.
    cached: Vec<LlamaToken>,
    stats: Stats,
}

enum PrefillErr {
    Cancelled,
    Backend(String),
}

impl State<'_> {
    /// Render the chat with the model's own template. A model without one
    /// falls back to ChatML (Qwen's format, also understood by most small
    /// instruct models).
    fn render(&self, p: &Prompt) -> Result<String, CleanErr> {
        if let Some(t) = &self.template {
            let msgs = p
                .messages
                .iter()
                .map(|(r, c)| LlamaChatMessage::new(r.clone(), c.clone()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| CleanErr::Backend(format!("chat message: {e}")))?;
            if let Ok(s) = self.model.apply_chat_template(t, &msgs, true) {
                return Ok(s);
            }
        }
        Ok(chatml(&p.messages))
    }

    fn tokenize(&self, text: &str) -> Vec<LlamaToken> {
        let vocab = self.model.vocab();
        let mut toks = vocab.tokenize(text.as_bytes(), true, true);
        // Templates that spell out BOS (Llama 3) plus add_special => two BOS.
        if toks.len() >= 2 && toks[0] == vocab.bos() && toks[1] == vocab.bos() {
            toks.remove(0);
        }
        toks
    }

    fn reset(&mut self) {
        self.ctx.clear_kv_cache();
        self.cached.clear();
    }

    /// Make the KV cache hold exactly `toks` and leave the logits of the
    /// last token ready for sampling. Only the suffix after the longest
    /// common prefix with the cache is decoded.
    fn prefill(&mut self, toks: &[LlamaToken], stop: &dyn Fn() -> bool) -> Result<(), PrefillErr> {
        let common = self
            .cached
            .iter()
            .zip(toks)
            .take_while(|(a, b)| a == b)
            .count();
        // The last token is always re-decoded so its logits exist.
        let mut keep = common.min(toks.len().saturating_sub(1));
        if self
            .ctx
            .kv_cache_seq_rm(0, Some(keep as u32), None)
            .is_err()
        {
            self.reset();
            keep = 0;
        }
        self.cached.truncate(keep);
        self.stats.prompt_tokens = toks.len();
        self.stats.prefill_tokens = toks.len() - keep;
        let t0 = Instant::now();
        let r = self.prefill_from(toks, keep, stop);
        self.stats.prefill_ms = t0.elapsed().as_millis() as u64;
        r
    }

    fn prefill_from(
        &mut self,
        toks: &[LlamaToken],
        keep: usize,
        stop: &dyn Fn() -> bool,
    ) -> Result<(), PrefillErr> {
        let mut batch = LlamaBatch::new(N_BATCH, 1);
        let mut pos = keep;
        for chunk in toks[keep..].chunks(N_BATCH) {
            if stop() {
                return Err(PrefillErr::Cancelled);
            }
            batch.clear();
            for (i, t) in chunk.iter().enumerate() {
                let last = pos + i == toks.len() - 1;
                batch
                    .add(*t, (pos + i) as i32, &[0], last)
                    .map_err(|e| PrefillErr::Backend(format!("batch: {e}")))?;
            }
            if let Err(e) = self.ctx.decode(&mut batch) {
                // Unknown KV state now; start clean next time.
                self.reset();
                return Err(PrefillErr::Backend(format!("prefill: {e}")));
            }
            self.cached.extend_from_slice(chunk);
            pos += chunk.len();
        }
        Ok(())
    }

    fn warm(&mut self, p: &Prompt) -> usize {
        let Ok(text) = self.render(p) else { return 0 };
        let toks = self.tokenize(&text);
        match self.prefill(&toks, &|| false) {
            Ok(()) => toks.len(),
            Err(_) => 0,
        }
    }

    fn run(
        &mut self,
        p: &Prompt,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, CleanErr> {
        let stop =
            || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) || Instant::now() >= deadline;
        if stop() {
            return Err(CleanErr::Cancelled);
        }
        let text = self.render(p)?;
        let toks = self.tokenize(&text);
        if toks.len() + 64 > N_CTX as usize {
            return Err(CleanErr::Backend(format!(
                "prompt too long ({} tokens)",
                toks.len()
            )));
        }
        match self.prefill(&toks, &stop) {
            Ok(()) => {}
            Err(PrefillErr::Cancelled) => return Err(CleanErr::Cancelled),
            Err(PrefillErr::Backend(e)) => return Err(CleanErr::Backend(e)),
        }
        let source = self
            .model
            .vocab()
            .tokenize(p.transcript.as_bytes(), false, false);
        let max_new = max_new_tokens(source.len()).min(N_CTX as usize - toks.len() - DRAFT_MAX - 1);
        let t0 = Instant::now();
        let out = self.generate(toks.len(), max_new, &source, &stop);
        self.stats.gen_ms = t0.elapsed().as_millis() as u64;
        // Drop the generated tokens from the cache; the prompt stays.
        if self
            .ctx
            .kv_cache_seq_rm(0, Some(toks.len() as u32), None)
            .is_err()
        {
            self.reset();
        }
        self.cached.truncate(toks.len());
        match out {
            Ok(Some(s)) => Ok(s),
            Ok(None) => Err(CleanErr::Cancelled),
            Err(e) => {
                self.reset();
                Err(e)
            }
        }
    }

    /// Greedy decode with prompt-lookup drafting. `Ok(None)` = stopped by
    /// deadline/cancel.
    ///
    /// Cleanup output mostly repeats the transcript, so after each token the
    /// longest n-gram match of the output tail in the transcript proposes
    /// up to [`DRAFT_MAX`] following tokens. They are decoded in the same
    /// batch as the real token and kept while they equal the greedy choice,
    /// so the result is exactly the greedy decode, in fewer forward passes.
    fn generate(
        &mut self,
        n_prompt: usize,
        max_new: usize,
        source: &[LlamaToken],
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<String>, CleanErr> {
        let vocab = self.model.vocab();
        let mut sampler = LlamaSampler::greedy();
        let mut bytes: Vec<u8> = Vec::new();
        let mut gen: Vec<LlamaToken> = Vec::new();
        let mut batch = LlamaBatch::new(DRAFT_MAX + 1, 1);
        let mut n_past = n_prompt;
        // -1 = logits of the last prefill token.
        let mut tok = sampler.sample(&self.ctx, -1);
        // Accept one token; false = generation is over.
        let accept = |t: LlamaToken, gen: &mut Vec<LlamaToken>, bytes: &mut Vec<u8>| -> bool {
            if vocab.is_eog(t) {
                return false;
            }
            gen.push(t);
            bytes.extend(vocab.token_to_piece(t, false, None));
            if let Some(cut) = find_stop(bytes) {
                bytes.truncate(cut);
                return false;
            }
            gen.len() < max_new
        };
        'outer: loop {
            if stop() {
                return Ok(None);
            }
            if !accept(tok, &mut gen, &mut bytes) {
                break;
            }
            let draft = lookup_draft(source, &gen, draft_max().min(max_new - gen.len()));
            batch.clear();
            for (j, t) in std::iter::once(&tok).chain(&draft).enumerate() {
                batch
                    .add(*t, (n_past + j) as i32, &[0], true)
                    .map_err(|e| CleanErr::Backend(format!("batch: {e}")))?;
            }
            self.ctx
                .decode(&mut batch)
                .map_err(|e| CleanErr::Backend(format!("decode: {e}")))?;
            self.stats.gen_steps += 1;
            let kv_end = n_past + 1 + draft.len();
            n_past += 1;
            let mut next = sampler.sample(&self.ctx, 0);
            for (j, d) in draft.iter().enumerate() {
                if next != *d {
                    break;
                }
                self.stats.draft_accepted += 1;
                n_past += 1;
                if !accept(*d, &mut gen, &mut bytes) {
                    break 'outer;
                }
                next = sampler.sample(&self.ctx, j as i32 + 1);
            }
            // Rejected draft positions leave the cache.
            if n_past < kv_end
                && self
                    .ctx
                    .kv_cache_seq_rm(0, Some(n_past as u32), None)
                    .is_err()
            {
                return Err(CleanErr::Backend("kv rollback failed".into()));
            }
            tok = next;
        }
        self.stats.gen_tokens = gen.len();
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// Longest draft: tokens decoded speculatively per forward pass.
const DRAFT_MAX: usize = 8;
/// Off by default: on the 4-thread i7-6700 CPU an n-token batch cost more
/// than the passes it saved. Drafts of 2/4/8 were 1.2-2x slower than plain
/// greedy for Qwen2.5 0.5B/1.5B Q4_K_M (tools/probes/llm_ab.sh). Kept,
/// exact-greedy, for GPU or faster-batch backends: `FLOW_LLM_DRAFT=n`.
const DRAFT_DEFAULT: usize = 0;

fn draft_max() -> usize {
    static D: OnceLock<usize> = OnceLock::new();
    *D.get_or_init(|| {
        std::env::var("FLOW_LLM_DRAFT")
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or(DRAFT_DEFAULT, |d: usize| d.min(DRAFT_MAX))
    })
}

/// Prompt-lookup draft: find the latest occurrence in `source` of the
/// longest (3..1) token suffix of `gen` and return what followed it.
pub fn lookup_draft(source: &[LlamaToken], gen: &[LlamaToken], max: usize) -> Vec<LlamaToken> {
    if max == 0 {
        return Vec::new();
    }
    for n in (1..=3usize.min(gen.len())).rev() {
        let tail = &gen[gen.len() - n..];
        if let Some(i) = source.windows(n).rposition(|w| w == tail) {
            let from = i + n;
            if from < source.len() {
                return source[from..(from + max).min(source.len())].to_vec();
            }
        }
    }
    Vec::new()
}

/// ChatML, for models without a usable embedded template.
pub fn chatml(messages: &[(String, String)]) -> String {
    let mut s = String::new();
    for (r, c) in messages {
        s.push_str(&format!("<|im_start|>{r}\n{c}<|im_end|>\n"));
    }
    s.push_str("<|im_start|>assistant\n");
    s
}

/// 03 §7.5: max_tokens = 2 × input tokens + 32.
pub fn max_new_tokens(input_tokens: usize) -> usize {
    2 * input_tokens + 32
}

fn find_stop(bytes: &[u8]) -> Option<usize> {
    STOP_STRINGS
        .iter()
        .filter_map(|s| bytes.windows(s.len()).position(|w| w == s.as_bytes()))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_strings_cut_at_earliest() {
        assert_eq!(find_stop(b"Hello.<|im_end|>junk"), Some(6));
        assert_eq!(find_stop(b"Hi.\nTRANSCRIPT: more\nAPP:"), Some(3));
        assert_eq!(find_stop(b"Plain text."), None);
    }

    #[test]
    fn chatml_fallback_renders_turns() {
        let m = vec![("system".into(), "S".into()), ("user".into(), "U".into())];
        assert_eq!(
            chatml(&m),
            "<|im_start|>system\nS<|im_end|>\n<|im_start|>user\nU<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    #[test]
    fn lookup_draft_continues_the_latest_longest_match() {
        let t = |v: &[i32]| v.iter().map(|&x| LlamaToken(x)).collect::<Vec<_>>();
        let src = t(&[1, 2, 3, 4, 5, 2, 3, 9]);
        // Suffix [2,3] last occurs at 5..7 -> [9].
        assert_eq!(lookup_draft(&src, &t(&[7, 2, 3]), 8), t(&[9]));
        // Suffix [4] -> [5, 2, 3], capped.
        assert_eq!(lookup_draft(&src, &t(&[4]), 3), t(&[5, 2, 3]));
        // No match, or match at the very end -> nothing.
        assert!(lookup_draft(&src, &t(&[42]), 8).is_empty());
        assert!(lookup_draft(&src, &t(&[9]), 8).is_empty());
        assert!(lookup_draft(&src, &t(&[4]), 0).is_empty());
    }

    #[test]
    fn budget_follows_03_section7() {
        assert_eq!(max_new_tokens(10), 52);
    }

    #[test]
    fn missing_model_is_a_load_error() {
        let e = Engine::load("/nonexistent/model.gguf", 2, None)
            .err()
            .unwrap();
        assert!(e.contains("not found"), "{e}");
    }
}
