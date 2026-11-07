//! whisper_joins probe (2026-11-07, ticket "Parity · Whisper repeats text at
//! segment joins"). Private data in, private data out: every path is an
//! argument and nothing here holds clip text.
//!
//! usage:
//!   joins core   '<ModelConfig JSON>' SESSION.json CLIPS_DIR OUT.jsonl
//!       FlowCore commit per clip, whole vs segment_pause_ms 800 (no
//!       pacing: pieces and bridges decode at commit, same text).
//!   joins pieces '<ModelConfig JSON>' SESSION.json CLIPS_DIR OUT.jsonl
//!       Segmenter + whisper directly: each piece decoded the way the
//!       core does (VAD trim, asr level), with and without the previous
//!       pieces' last words as prior_text, plus the cut sample indices.
//!       `whisper_fit_ctx` in the ModelConfig picks the encoder window.
use flow_core::asr::{compose_prompt, prior_words, AsrEngine, AsrPurpose, AsrRequest};
use flow_core::config::{ModelConfig, SessionConfig};
use flow_core::segment::{SegmentParams, Segmenter};
use flow_core::FlowCore;
use std::io::Write;

fn clips(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    v.sort();
    v
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model: ModelConfig = serde_json::from_str(&a[2]).unwrap();
    let base: SessionConfig =
        serde_json::from_str(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let dir = std::path::PathBuf::from(&a[4]);
    let only: Vec<String> = a[6..].to_vec();
    let mut out = std::fs::File::create(&a[5]).unwrap();
    let wanted = |p: &std::path::Path| {
        let n = p.file_stem().unwrap().to_string_lossy().to_string();
        (only.is_empty() || only.contains(&n)).then_some(n)
    };
    match a[1].as_str() {
        "core" => {
            let core = FlowCore::load_models(&model).expect("load");
            for p in clips(&dir) {
                let Some(name) = wanted(&p) else { continue };
                let pcm = flow_core::audio::read_wav_16k_mono(&p).unwrap();
                for (mode, seg) in [("whole", None), ("seg", Some(800u32))] {
                    let mut cfg = base.clone();
                    cfg.segment_pause_ms = seg;
                    cfg.partial_interval_ms = None;
                    cfg.ring_capacity_samples = cfg.ring_capacity_samples.max(pcm.len() + 16_000);
                    core.start_session(cfg).unwrap();
                    for c in pcm.chunks(1600) {
                        core.push_audio(c);
                    }
                    let r = core.commit();
                    writeln!(
                        out,
                        "{}",
                        serde_json::json!({
                        "clip": name, "mode": mode, "asr_text": r.asr_text,
                        "final_text": r.final_text, "segments": r.timings.segments,
                        "asr_ms": r.timings.asr_ms, "status": format!("{:?}", r.status)})
                    )
                    .unwrap();
                    eprintln!("{name} {mode} segments={:?}", r.timings.segments);
                }
            }
        }
        "pieces" => {
            let w = flow_core::whisper::WhisperEngine::load(&model.asr_path, model.threads.max(1))
                .expect("whisper")
                .with_fit_ctx(model.whisper_fit_ctx);
            let mut cfg = base.clone();
            cfg.segment_pause_ms = Some(800);
            let params = SegmentParams::for_session(&cfg).unwrap();
            let prompt = flow_core::asr::build_initial_prompt(&cfg.dictionary, &[]);
            for p in clips(&dir) {
                let Some(name) = wanted(&p) else { continue };
                let pcm: Vec<f32> = flow_core::audio::read_wav_16k_mono(&p)
                    .unwrap()
                    .iter()
                    .map(|&s| s as f32 / 32768.0)
                    .collect();
                let mut sg = Segmenter::new(params);
                sg.push(&pcm);
                let mut bounds = vec![0usize];
                bounds.extend_from_slice(sg.cuts());
                bounds.push(pcm.len());
                let (mut with_prior, mut no_prior) = (Vec::<String>::new(), Vec::<String>::new());
                let mut prompts = Vec::new();
                for k in 0..bounds.len() - 1 {
                    let piece = &pcm[bounds[k]..bounds[k + 1]];
                    let Some(sp) = flow_core::vad::detect(piece) else {
                        with_prior.push(String::new());
                        no_prior.push(String::new());
                        prompts.push(0);
                        continue;
                    };
                    let mut lvl = flow_core::vad::asr_level(&piece[sp.start..sp.end]).into_owned();
                    // PIECE_MIN_MS: zero-pad the piece to at least this long,
                    // i.e. a floor on the fitted encoder window (whisper pads
                    // to 30 s with zeros anyway; audio_ctx only cuts it).
                    if let Some(ms) = std::env::var("PIECE_MIN_MS")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                    {
                        lvl.resize(lvl.len().max(ms * 16), 0.0);
                    }
                    let mut req = AsrRequest {
                        initial_prompt: prompt.clone(),
                        language: Some("en".into()),
                        purpose: AsrPurpose::Segment,
                        ..Default::default()
                    };
                    no_prior.push(w.transcribe(&lvl, &req).unwrap().text);
                    req.prior_text = prior_words(&with_prior.join(" "));
                    prompts.push(
                        compose_prompt(&req.initial_prompt, req.prior_text.as_deref())
                            .split_whitespace()
                            .count(),
                    );
                    with_prior.push(w.transcribe(&lvl, &req).unwrap().text);
                }
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                    "clip": name, "cuts": sg.cuts(), "prompt_words": prompts,
                    "with_prior": with_prior, "no_prior": no_prior})
                )
                .unwrap();
                eprintln!("{name} pieces={}", bounds.len() - 1);
            }
        }
        m => panic!("unknown mode {m}"),
    }
}
