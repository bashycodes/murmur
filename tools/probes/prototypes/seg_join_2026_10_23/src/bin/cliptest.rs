//! cliptest (copy of the 2026-10-23 cliptest harness, fix/seg-polish): each clip pushed in 100 ms frames at real-time pace with
//! previews on (partial_interval_ms 1500, like the phone), whole vs
//! segment_pause_ms 800, then times commit (release -> text).
//! usage: cliptest '<ModelConfig JSON>' '<languages JSON>' clips_dir name...
use std::time::{Duration, Instant};
use flow_core::config::{ModelConfig, SessionConfig};
use flow_core::FlowCore;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model: ModelConfig = serde_json::from_str(&a[1]).unwrap();
    let langs: Vec<String> = serde_json::from_str(&a[2]).unwrap();
    let dir = std::path::PathBuf::from(&a[3]);
    let t = Instant::now();
    let core = FlowCore::load_models(&model).expect("load");
    eprintln!("load {} ms", t.elapsed().as_millis());
    for name in &a[4..] {
        let pcm = flow_core::audio::read_wav_16k_mono(&dir.join(format!("{name}.wav"))).unwrap();
        for (mode, seg) in [("whole", None), ("seg", Some(800u32))] {
            let mut cfg: SessionConfig = serde_json::from_value(serde_json::json!({"partial_interval_ms": 1500})).unwrap();
            cfg.languages = langs.clone();
            cfg.segment_pause_ms = seg;
            cfg.ring_capacity_samples = cfg.ring_capacity_samples.max(pcm.len() + 16_000);
            core.start_session(cfg).unwrap();
            let t0 = Instant::now();
            let mut npoll = 0usize;
            for (i, c) in pcm.chunks(1600).enumerate() {
                core.push_audio(c);
                let due = Duration::from_millis(100 * (i as u64 + 1));
                // CLIPTEST_FAST=1: no real-time pacing (same text; the
                // pieces and bridges are decoded at commit instead).
                if std::env::var_os("CLIPTEST_FAST").is_none() {
                    if let Some(w) = due.checked_sub(t0.elapsed()) { std::thread::sleep(w); }
                }
                npoll += 1;
            }
            let t1 = Instant::now();
            let r = core.commit();
            let ms = t1.elapsed().as_millis();
            println!("{}", serde_json::json!({
                "clip": name, "mode": mode, "commit_ms": ms, "status": format!("{:?}", r.status),
                "asr_text": r.asr_text, "final_text": r.final_text, "error": r.error,
                "empty_reason": r.empty_reason,
                "t": {"asr": r.timings.asr_ms, "total": r.timings.total_ms, "wait": r.timings.wait_ms,
                       "segments": r.timings.segments, "pre_asr": r.timings.pre_asr_ms,
                       "audio": r.timings.audio_ms, "dropped": r.timings.dropped_samples},
                "frames": npoll
            }));
        }
    }
}
