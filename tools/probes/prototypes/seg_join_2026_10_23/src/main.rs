//! Probe (2026-10-23, fix/seg-polish): print each D22 piece, each bridge
//! decode and the splice result for a clip, offline (no session, no
//! pacing), mirroring `MurmurCore::commit_segmented`'s piece/bridge ranges.
//! usage: seg-join-probe parakeet|whisper <model> <wav> [pause_ms]
use murmur_core::asr::{AsrEngine, AsrRequest};
use murmur_core::segment::{self, SegmentParams, Segmenter};
use murmur_core::vad;

fn dec(asr: &dyn AsrEngine, pcm: &[f32]) -> String {
    let Some(sp) = vad::detect(pcm) else { return String::new() };
    let req = AsrRequest { language: Some("en".into()), ..Default::default() };
    asr.transcribe(&pcm[sp.start..sp.end], &req).unwrap().text.trim().to_string()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let asr: Box<dyn AsrEngine> = match a[1].as_str() {
        "parakeet" => Box::new(murmur_core::parakeet::ParakeetAsr::load(&a[2], 4).unwrap()),
        _ => Box::new(murmur_core::whisper::WhisperEngine::load(&a[2], 4).unwrap()),
    };
    let pcm: Vec<f32> = murmur_core::audio::read_wav_16k_mono(std::path::Path::new(&a[3])).unwrap().iter().map(|&s| s as f32 / 32768.0).collect();
    let pause = a.get(4).map_or(800, |p| p.parse().unwrap());
    let mut sg = Segmenter::new(SegmentParams { pause_ms: pause, min_ms: 4000, soft_max_ms: 20000 });
    for c in pcm.chunks(1600) { sg.push(c); }
    let mut b = vec![0];
    b.extend_from_slice(sg.cuts());
    b.push(pcm.len());
    println!("cuts (s): {:?}", sg.cuts().iter().map(|c| *c as f32 / 16000.0).collect::<Vec<_>>());
    println!("WHOLE: {}", dec(&*asr, &pcm));
    let mut t: Vec<String> = b.windows(2).map(|w| dec(&*asr, &pcm[w[0]..w[1]])).collect();
    for (k, x) in t.iter().enumerate() { println!("PIECE {k}: {x}"); }
    for k in 0..t.len().saturating_sub(1) {
        let r = segment::bridge_range(b[k + 1], b[k], b[k + 2]);
        let bt = dec(&*asr, &pcm[r.clone()]);
        println!("BRIDGE {k} {:.1}-{:.1}s: {bt}", r.start as f32 / 16e3, r.end as f32 / 16e3);
        match segment::splice(&t[k], &t[k + 1], &bt) {
            Some((l, rr)) => { println!("  SPLICE -> ...{} | {}...", tail(&l), head(&rr)); t[k] = l; t[k + 1] = rr; }
            None => println!("  SPLICE -> None"),
        }
    }
    println!("JOINED: {}", t.join(" "));
    if std::env::var("EXP").is_ok() { exp(&*asr, &pcm, &b); }
}

/// Variants: piece end padding, and wider bridges.
fn exp(asr: &dyn AsrEngine, pcm: &[f32], b: &[usize]) {
    let req = AsrRequest { language: Some("en".into()), ..Default::default() };
    let raw = |x: &[f32]| asr.transcribe(x, &req).unwrap().text.trim().to_string();
    let last = |s: String| { let w: Vec<_> = s.split_whitespace().collect(); w[w.len().saturating_sub(4)..].join(" ") };
    for w in b.windows(2) {
        let p = &pcm[w[0]..w[1]];
        let sp = vad::detect(p).unwrap();
        let tr = &p[sp.start..sp.end];
        let mut pad = tr.to_vec();
        pad.extend(std::iter::repeat(0.0).take(16000));
        let mut pad2 = tr.to_vec();
        pad2.extend(std::iter::repeat(0.0).take(32000));
        println!("piece {:.1}-{:.1}: trim[{}] untrim[{}] +1s0[{}] +2s0[{}]", w[0] as f32/16e3, w[1] as f32/16e3,
            last(raw(tr)), last(raw(p)), last(raw(&pad)), last(raw(&pad2)));
    }
    for k in 0..b.len().saturating_sub(2) {
        for (l, rr) in [(4usize, 4usize), (6, 4), (8, 4), (10, 4), (6, 6), (8, 8)] {
            let c = b[k + 1];
            let r = c.saturating_sub(l * 16000).max(b[k])..(c + rr * 16000).min(b[k + 2]);
            println!("bridge {k} -{l}/+{rr}s: {}", dec(asr, &pcm[r]));
        }
    }
}
fn tail(s: &str) -> String { let w: Vec<_> = s.split_whitespace().collect(); w[w.len().saturating_sub(6)..].join(" ") }
fn head(s: &str) -> String { s.split_whitespace().take(6).collect::<Vec<_>>().join(" ") }
