//! Dated probe 2026-10-23 (docs/proposals/stt-model-survey.md).
//!
//! stt_survey KIND MODEL_DIR MANIFEST AUDIO_DIR [--threads N] [--beam N]
//!            [--long SECONDS,...] [--reps N] [--no-clips] [--chunk-ms N]
//!
//! KIND: transducer (Parakeet TDT / unified, nemo_transducer), canary,
//! moonshine2, qwen3, cohere, online (streaming NeMo transducer, e.g.
//! Nemotron streaming; audio fed in --chunk-ms chunks at max speed, tail
//! latency = time from the last chunk to the final text).
//!
//! One JSON line per decode on stdout: {kind:"clip"|"long", id, ref, hyp,
//! dur_s, infer_ms, cpu_ms (process user+sys during the decode), cores
//! (= cpu_ms / infer_ms)}. Long clips are LibriSpeech manifest clips
//! concatenated (0.3 s gap) up to each target length; each is decoded
//! --reps times so first-shape effects show. Score with score.py.

use std::path::{Path, PathBuf};
use std::time::Instant;

use sherpa_onnx::*;

fn find(dir: &Path, must: &[&str]) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| must.iter().all(|m| n.contains(m)))
        .collect();
    names.sort();
    // prefer int8 files
    let pick = names
        .iter()
        .find(|n| n.contains("int8"))
        .or(names.first())?
        .clone();
    Some(dir.join(pick).to_string_lossy().into_owned())
}

fn cpu_ms() -> f64 {
    unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut r);
        let t = |tv: libc::timeval| tv.tv_sec as f64 * 1e3 + tv.tv_usec as f64 / 1e3;
        t(r.ru_utime) + t(r.ru_stime)
    }
}

enum Rec {
    Off(OfflineRecognizer),
    On(OnlineRecognizer, usize),
}

impl Rec {
    fn run(&self, pcm: &[f32]) -> (String, f64, f64) {
        match self {
            Rec::Off(r) => {
                let t = Instant::now();
                let s = r.create_stream();
                s.accept_waveform(16000, pcm);
                r.decode(&s);
                let txt = s.get_result().map(|x| x.text).unwrap_or_default();
                (txt, t.elapsed().as_secs_f64() * 1e3, 0.0)
            }
            Rec::On(r, chunk) => {
                // Feed chunks and decode as they become ready (max speed;
                // in real use this overlaps with speaking). Then measure the
                // tail: input_finished + padding + drain.
                let s = r.create_stream();
                let t0 = Instant::now();
                for c in pcm.chunks(*chunk) {
                    s.accept_waveform(16000, c);
                    while r.is_ready(&s) {
                        r.decode(&s);
                    }
                }
                let feed = t0.elapsed().as_secs_f64() * 1e3;
                let t = Instant::now();
                s.accept_waveform(16000, &vec![0.0f32; 16000 * 8 / 10]);
                s.input_finished();
                while r.is_ready(&s) {
                    r.decode(&s);
                }
                let txt = r.get_result(&s).map(|x| x.text).unwrap_or_default();
                (txt, t.elapsed().as_secs_f64() * 1e3, feed)
            }
        }
    }
}

fn load(kind: &str, dir: &Path, threads: i32, beam: i32) -> Rec {
    let tokens = find(dir, &["tokens.txt"]);
    if kind == "online" {
        let mut c = OnlineRecognizerConfig::default();
        c.model_config.transducer = OnlineTransducerModelConfig {
            encoder: find(dir, &["encoder", ".onnx"]),
            decoder: find(dir, &["decoder", ".onnx"]),
            joiner: find(dir, &["joiner", ".onnx"]),
        };
        c.model_config.tokens = tokens;
        c.model_config.num_threads = threads;
        c.decoding_method = Some("greedy_search".into());
        eprintln!("{:?}", c.model_config.transducer);
        let r = OnlineRecognizer::create(&c).expect("create online");
        return Rec::On(r, 16000 / 10);
    }
    let mut c = OfflineRecognizerConfig::default();
    c.model_config.tokens = tokens;
    c.model_config.num_threads = threads;
    c.model_config.provider = Some("cpu".into());
    match kind {
        "transducer" => {
            c.model_config.transducer = OfflineTransducerModelConfig {
                encoder: find(dir, &["encoder", ".onnx"]),
                decoder: find(dir, &["decoder", ".onnx"]),
                joiner: find(dir, &["joiner", ".onnx"]),
            };
            c.model_config.model_type = Some("nemo_transducer".into());
            if beam > 1 {
                c.decoding_method = Some("modified_beam_search".into());
                c.max_active_paths = beam;
            } else {
                c.decoding_method = Some("greedy_search".into());
            }
        }
        "canary" => {
            c.model_config.canary = OfflineCanaryModelConfig {
                encoder: find(dir, &["encoder", ".onnx"]),
                decoder: find(dir, &["decoder", ".onnx"]),
                src_lang: Some("en".into()),
                tgt_lang: Some("en".into()),
                use_pnc: true,
            };
        }
        "moonshine2" => {
            c.model_config.moonshine = OfflineMoonshineModelConfig {
                encoder: find(dir, &["encoder", ".ort"]).or(find(dir, &["encoder", ".onnx"])),
                merged_decoder: find(dir, &["decoder", ".ort"])
                    .or(find(dir, &["decoder", ".onnx"])),
                ..Default::default()
            };
        }
        "qwen3" => {
            let tok = dir.join("tokenizer");
            c.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
                conv_frontend: find(dir, &["conv_frontend", ".onnx"]),
                encoder: find(dir, &["encoder", ".onnx"]),
                decoder: find(dir, &["decoder", ".onnx"]),
                tokenizer: Some(if tok.is_dir() { tok } else { dir.to_path_buf() }
                    .to_string_lossy()
                    .into_owned()),
                ..Default::default()
            };
        }
        "cohere" => {
            c.model_config.cohere_transcribe = OfflineCohereTranscribeModelConfig {
                encoder: find(dir, &["encoder", ".onnx"]),
                decoder: find(dir, &["decoder", ".onnx"]),
                language: Some("en".into()),
                use_punct: true,
                use_itn: true,
            };
        }
        k => panic!("unknown kind {k}"),
    }
    let r = OfflineRecognizer::create(&c).expect("create offline (see stderr)");
    Rec::Off(r)
}

fn read_wav(p: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    assert_eq!(r.spec().sample_rate, 16000, "{}", p.display());
    r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (kind, dir, manifest, audio) =
        (a[0].as_str(), PathBuf::from(&a[1]), &a[2], PathBuf::from(&a[3]));
    let (mut threads, mut beam, mut reps, mut clips) = (4, 1, 2, true);
    let mut long: Vec<f64> = vec![];
    let mut i = 4;
    while i < a.len() {
        match a[i].as_str() {
            "--threads" => { i += 1; threads = a[i].parse().unwrap() }
            "--beam" => { i += 1; beam = a[i].parse().unwrap() }
            "--reps" => { i += 1; reps = a[i].parse().unwrap() }
            "--no-clips" => clips = false,
            "--long" => { i += 1; long = a[i].split(',').map(|x| x.parse().unwrap()).collect() }
            o => panic!("unknown option {o}"),
        }
        i += 1;
    }
    let t = Instant::now();
    let rec = load(kind, &dir, threads, beam);
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    let utts = m["utterances"].as_array().unwrap().clone();
    let (w, _, _) = rec.run(&read_wav(&audio.join("jfk.wav"))); // warm-up
    eprintln!("load_ms={load_ms:.0} warmup={w:?}");
    let emit = |k: &str, id: &str, r: &str, pcm: &[f32]| {
        let c0 = cpu_ms();
        let (hyp, ms, feed_ms) = rec.run(pcm);
        let c = cpu_ms() - c0;
        println!("{}", serde_json::json!({"kind": k, "id": id, "ref": r, "hyp": hyp.trim(),
            "dur_s": pcm.len() as f64 / 16000.0, "infer_ms": ms, "cpu_ms": c,
            "cores": c / ms.max(1e-3), "feed_ms": feed_ms, "threads": threads, "beam": beam, "model": kind,
            "dir": dir.file_name().unwrap().to_string_lossy(), "load_ms": load_ms}));
    };
    if clips {
        for u in &utts {
            let id = u["id"].as_str().unwrap();
            emit("clip", id, u["ref"].as_str().unwrap(), &read_wav(&audio.join(format!("{id}.wav"))));
        }
    }
    // Long clips: shortest utterances first, so lengths land near the target;
    // whole utterances only (refs stay exact); actual length is in dur_s.
    let mut by_len = utts.clone();
    by_len.sort_by(|x, y| x["dur_s"].as_f64().partial_cmp(&y["dur_s"].as_f64()).unwrap());
    for target in long {
        let (mut pcm, mut refs) = (Vec::new(), Vec::new());
        for u in &by_len {
            let have = pcm.len() as f64 / 16000.0;
            if have >= target - 1.0 { break; }
            if have + u["dur_s"].as_f64().unwrap() + 0.3 > target + 1.0 { continue; }
            let id = u["id"].as_str().unwrap();
            pcm.extend(read_wav(&audio.join(format!("{id}.wav"))));
            pcm.extend(std::iter::repeat(0.0).take(4800));
            refs.push(u["ref"].as_str().unwrap().to_string());
        }
        for r in 0..reps {
            emit("long", &format!("long{target}s-rep{r}"), &refs.join(" "), &pcm);
        }
    }
}
