//! murmur-replay: run saved recordings through speech models x cleanup
//! models x prompt versions (see the crate docs and tools/replay/README.md).
//!
//! murmur-replay --in CORPUS --out DIR [--asr LIST] [--cleanup LIST]
//!   [--prompt LIST] [--max-spend USD] [--session FILE] [--threads N]
//!   [--llm GGUF] [--llm-lib SO] [--key-file FILE] [--no-local-fallback]
//!   [--only ID,...]
//!
//! * `--asr`: `parakeet[=DIR]` (default `$MURMUR_PARAKEET_DIR`),
//!   `whisper[=GGML]` (default `$MURMUR_WHISPER_MODEL`), `or-stt[=MODEL]`
//!   (OpenRouter cloud STT, default the product's whisper-large-v3-turbo).
//!   Default `parakeet`.
//! * `--cleanup`: `none` (fallback rules), `local` (the on-device pipeline:
//!   rules, Qwen prompt v2; needs `--llm`), `or:MODEL`
//!   (OpenRouter AI cleanup, D30; with `--llm` it falls back to the local
//!   pipeline as on the phone unless `--no-local-fallback`). Default
//!   `none,or:meta-llama/llama-3.3-70b-instruct`.
//! * `--prompt`: versions for the `or:` slots, `v5` (the product), `v4c`
//!   and/or `v2`. Default `v5`.
//! * `--max-spend`: cap on OpenRouter spend for this run (default 0.05).
//! * `--only`: recordings whose id contains one of these (smoke runs).
//!
//! Writes `DIR/rows.jsonl` (one row per combination), `DIR/recordings.jsonl`
//! (the corpus with the phone's results) and `DIR/run.json` (matrix, spend).
//! The OpenRouter key is read from the key file (default
//! `$HOME/.config/murmur/openrouter.key`) only when an OpenRouter slot
//! is asked for, and goes only into the request header: it is never
//! printed, logged or written.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use murmur_core::cleanup::Cleaner;
use murmur_core::config::SessionConfig;
use murmur_replay::{
    load_corpus, parse_prompt, phone_session, prompt_name, AsrSlot, CleanupSlot, Matrix,
};
use murmur_router::SecretString;

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("murmur-replay: {msg}");
    std::process::exit(2)
}

struct Args(Vec<String>);

impl Args {
    fn get(&self, k: &str) -> Option<String> {
        self.0
            .iter()
            .position(|a| a == k)
            .and_then(|i| self.0.get(i + 1).cloned())
    }
    fn has(&self, k: &str) -> bool {
        self.0.iter().any(|a| a == k)
    }
    fn list(&self, k: &str, default: &str) -> Vec<String> {
        self.get(k)
            .unwrap_or_else(|| default.into())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

fn env_or(arg: Option<&str>, var: &str, what: &str) -> String {
    arg.map(str::to_string)
        .or_else(|| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| die(format!("{what}: pass it after '=' or set {var}")))
}

/// The key, read once from the file; never printed.
fn load_key(a: &Args) -> SecretString {
    let path = a.get("--key-file").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".config/murmur/openrouter.key")
    });
    let k = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        die(format!(
            "OpenRouter key file {}: {}",
            path.display(),
            e.kind()
        ))
    });
    let k = SecretString::new(k.trim().to_string());
    if k.is_empty() {
        die(format!("OpenRouter key file {} is empty", path.display()));
    }
    k
}

fn key_usage(key: &SecretString) -> Option<f64> {
    murmur_cloud::keyinfo::fetch(key).ok().and_then(|k| k.usage)
}

fn asr_slot(spec: &str, threads: usize, key: &dyn Fn() -> SecretString) -> AsrSlot {
    let (kind, arg) = match spec.split_once('=') {
        Some((k, v)) => (k, Some(v)),
        None => (spec, None),
    };
    let t = Instant::now();
    let slot = match kind {
        #[cfg(feature = "parakeet")]
        "parakeet" => {
            let dir = env_or(arg, "MURMUR_PARAKEET_DIR", "parakeet model dir");
            let e =
                murmur_core::parakeet::ParakeetAsr::load_with(&dir, threads, Default::default())
                    .unwrap_or_else(|e| die(format!("parakeet {dir}: {e}")));
            let name = PathBuf::from(&dir)
                .file_name()
                .map_or("parakeet".into(), |n| n.to_string_lossy().into_owned());
            AsrSlot {
                name: format!(
                    "parakeet:{}",
                    name.trim_start_matches("sherpa-onnx-nemo-")
                        .trim_start_matches("parakeet-")
                ),
                engine: Arc::new(e),
                call_cost: None,
            }
        }
        #[cfg(feature = "whisper")]
        "whisper" => {
            let path = env_or(arg, "MURMUR_WHISPER_MODEL", "whisper ggml model");
            let e = murmur_core::whisper::WhisperEngine::load(&path, threads)
                .unwrap_or_else(|e| die(format!("whisper {path}: {e}")))
                .with_fit_ctx(true);
            let name = PathBuf::from(&path)
                .file_stem()
                .map_or("whisper".into(), |n| n.to_string_lossy().into_owned());
            AsrSlot {
                name: format!("whisper:{}", name.trim_start_matches("ggml-")),
                engine: Arc::new(e),
                call_cost: None,
            }
        }
        "or-stt" => {
            let settings = murmur_cloud::SttSettings {
                model: arg.unwrap_or(murmur_cloud::DEFAULT_STT_MODEL).to_string(),
                ..Default::default()
            };
            let name = format!("or-stt:{}", settings.model);
            let asr = Arc::new(murmur_cloud::CloudAsr::new(settings, key()));
            let a2 = asr.clone();
            AsrSlot {
                name,
                engine: asr,
                call_cost: Some(Arc::new(move || {
                    a2.last_call().and_then(|c| c.cost_usd).or(Some(0.0))
                })),
            }
        }
        k => die(format!(
            "unknown or not compiled in speech model {k:?} (build features: parakeet, whisper)"
        )),
    };
    eprintln!("asr {} loaded in {} ms", slot.name, t.elapsed().as_millis());
    slot
}

fn main() {
    let a = Args(std::env::args().skip(1).collect());
    if a.has("--help") || a.has("-h") {
        eprintln!(
            "{}",
            include_str!("main.rs")
                .lines()
                .take_while(|l| l.starts_with("//!"))
                .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                .collect::<Vec<_>>()
                .join("\n")
        );
        return;
    }
    let input = PathBuf::from(a.get("--in").unwrap_or_else(|| die("--in CORPUS_DIR")));
    let out_dir = PathBuf::from(a.get("--out").unwrap_or_else(|| die("--out DIR")));
    let threads: usize = a
        .get("--threads")
        .map_or(4, |t| t.parse().unwrap_or_else(|_| die("--threads N")));
    let max_spend: f64 = a.get("--max-spend").map_or(0.05, |t| {
        t.parse().unwrap_or_else(|_| die("--max-spend USD"))
    });
    let asr_specs = a.list("--asr", "parakeet");
    let cleanup_specs = a.list("--cleanup", "none,or:meta-llama/llama-3.3-70b-instruct");
    let prompts: Vec<_> = a
        .list("--prompt", "v5")
        .iter()
        .map(|p| parse_prompt(p).unwrap_or_else(|e| die(e)))
        .collect();
    let session: SessionConfig = match a.get("--session") {
        Some(p) => serde_json::from_str(
            &std::fs::read_to_string(&p).unwrap_or_else(|e| die(format!("{p}: {e}"))),
        )
        .unwrap_or_else(|e| die(format!("{p}: {e}"))),
        None => phone_session(),
    };
    let mut recs = load_corpus(&input).unwrap_or_else(|e| die(e));
    if let Some(only) = a.get("--only") {
        let ids: Vec<&str> = only.split(',').map(str::trim).collect();
        recs.retain(|r| ids.iter().any(|i| r.id.contains(i)));
    }
    if recs.is_empty() {
        die(format!("no recordings in {}", input.display()));
    }
    std::fs::create_dir_all(&out_dir)
        .unwrap_or_else(|e| die(format!("{}: {e}", out_dir.display())));

    let paid = asr_specs.iter().any(|s| s.starts_with("or-stt"))
        || cleanup_specs.iter().any(|s| s.starts_with("or:"));
    let key = paid.then(|| load_key(&a));
    let key_fn = || key.clone().unwrap_or_else(|| die("no key"));
    let usage_start = key.as_ref().and_then(key_usage);

    // The on-device LLM, loaded once (memory rules: run.sh holds the lock).
    let wants_local = cleanup_specs.iter().any(|s| s == "local");
    let local: Option<Arc<dyn Cleaner>> = match a.get("--llm") {
        None if wants_local => die("--cleanup local needs --llm GGUF"),
        None => None,
        #[cfg(feature = "llm")]
        Some(m) => {
            let t = Instant::now();
            let c =
                murmur_core::llama::LlamaCleaner::load(&m, threads, a.get("--llm-lib").as_deref())
                    .unwrap_or_else(|e| die(format!("llm {m}: {e}")));
            eprintln!("llm loaded in {} ms", t.elapsed().as_millis());
            Some(Arc::new(c))
        }
        #[cfg(not(feature = "llm"))]
        Some(_) => die("--llm needs a build with the llm feature"),
    };
    let fallback = if a.has("--no-local-fallback") {
        None
    } else {
        local.clone()
    };

    let asr: Vec<AsrSlot> = asr_specs
        .iter()
        .map(|s| asr_slot(s, threads, &key_fn))
        .collect();
    let cleanup: Vec<CleanupSlot> = cleanup_specs
        .iter()
        .map(|s| match s.as_str() {
            "none" => CleanupSlot {
                name: "none".into(),
                cloud: None,
                local: None,
            },
            "local" => CleanupSlot {
                name: "local".into(),
                cloud: None,
                local: local.clone(),
            },
            o if o.starts_with("or:") => {
                let model = o.trim_start_matches("or:").to_string();
                if !murmur_cloud::settings::valid_model_id(&model) {
                    die(format!("bad model id {model:?}"));
                }
                let settings = murmur_cloud::CleanupSettings {
                    model,
                    ..Default::default()
                };
                let name = if fallback.is_some() {
                    format!("{o}+local")
                } else {
                    o.to_string()
                };
                let cl = murmur_cloud::OpenRouterCleaner::new(settings, key_fn());
                CleanupSlot {
                    name,
                    cloud: Some(Arc::new(cl)),
                    local: fallback.clone(),
                }
            }
            o => die(format!("unknown cleanup {o:?} (none, local, or:MODEL)")),
        })
        .collect();

    let m = Matrix {
        asr,
        cleanup,
        prompts: prompts.clone(),
        session,
        max_spend,
    };
    let mut rows_f = std::fs::File::create(out_dir.join("rows.jsonl")).unwrap_or_else(|e| die(e));
    let mut recs_f =
        std::fs::File::create(out_dir.join("recordings.jsonl")).unwrap_or_else(|e| die(e));
    for r in &recs {
        writeln!(recs_f, "{}", serde_json::to_string(r).unwrap()).unwrap();
    }
    let t = Instant::now();
    let mut n = 0usize;
    let spend = m.run(
        &recs,
        &mut |r| murmur_core::audio::read_wav_16k_mono(&r.wav),
        &mut |row| {
            n += 1;
            writeln!(rows_f, "{}", serde_json::to_string(row).unwrap()).unwrap();
            eprintln!(
                "{:>3} {} {} {} {}: {} {}",
                n,
                row.rec,
                row.asr,
                row.cleanup,
                row.prompt,
                row.cleanup_route.as_deref().unwrap_or("-"),
                row.error
                    .as_deref()
                    .or(row.cloud_fallback.as_deref())
                    .unwrap_or("")
            );
        },
    );
    let mut usage_delta = None;
    if let (Some(k), Some(u0)) = (&key, usage_start) {
        // The key's usage counter lags the calls a little.
        std::thread::sleep(Duration::from_secs(5));
        usage_delta = key_usage(k).map(|u1| u1 - u0);
    }
    let run = serde_json::json!({
        "recordings": recs.len(), "rows": n, "elapsed_ms": t.elapsed().as_millis() as u64,
        "asr": m.asr.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "cleanup": m.cleanup.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "prompts": prompts.iter().map(|p| prompt_name(*p)).collect::<Vec<_>>(),
        "max_spend_usd": max_spend,
        "spend_usd": {"asr": spend.asr, "cleanup": spend.cleanup, "total": spend.total(),
                      "key_usage_delta": usage_delta},
        "murmur_core": murmur_core::VERSION, "postprocess_rev": murmur_core::POSTPROCESS_REV,
    });
    std::fs::write(
        out_dir.join("run.json"),
        serde_json::to_string_pretty(&run).unwrap() + "\n",
    )
    .unwrap();
    eprintln!(
        "openrouter spend: ${:.5} (stt ${:.5}, cleanup ${:.5}){}",
        spend.total(),
        spend.asr,
        spend.cleanup,
        usage_delta.map_or(String::new(), |d| format!("; key usage delta ${d:.5}"))
    );
    eprintln!("rows: {}", out_dir.join("rows.jsonl").display());
}
