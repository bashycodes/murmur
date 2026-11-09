//! Host-side check of the commit-time speech gate on wav files.
//!
//! cargo run -p murmur-core --features wav --example vad_gate -- [--dump DIR] WAV...
//!
//! Prints, per clip, what a commit with VAD on would do before ASR:
//! `no_speech` (the VAD found none), `blank_asr` (the VAD passed but the
//! audio handed to the ASR is still below the engines' 0.003 RMS silence
//! gate, so they return "") or `asr` (the ASR gets to decode it). Also the
//! clip RMS, speech_ms, and the RMS of what the ASR receives. `--dump DIR`
//! writes that ASR input as DIR/<clip>.wav (16-bit) for decoding elsewhere.

use murmur_core::vad;

/// The whole-clip RMS gate inside the sherpa and whisper engines.
const ENGINE_SILENCE_RMS: f32 = 0.003;

fn rms(p: &[f32]) -> f32 {
    if p.is_empty() {
        return 0.0;
    }
    (p.iter().map(|s| s * s).sum::<f32>() / p.len() as f32).sqrt()
}

fn read_wav(p: &str) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    assert_eq!(r.spec().sample_rate, 16_000, "{p}: needs 16 kHz");
    match r.spec().sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    }
}

fn main() {
    println!("clip\tclip_rms_pcm16\tdecision\tspeech_ms\tasr_rms_pcm16\tspan_ms");
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dump = match args.iter().position(|a| a == "--dump") {
        Some(i) => {
            let d = args.remove(i + 1);
            args.remove(i);
            std::fs::create_dir_all(&d).unwrap();
            Some(d)
        }
        None => None,
    };
    for p in args {
        let pcm = read_wav(&p);
        let name = std::path::Path::new(&p)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let span = vad::detect(&pcm).map_or("-".into(), |sp| {
            format!("{}-{}", sp.start / 16, sp.end / 16)
        });
        let (decision, speech_ms, asr_rms) = match vad::detect(&pcm) {
            None => ("no_speech", 0, 0.0),
            Some(sp) => {
                let asr = vad::asr_level(&pcm[sp.start..sp.end]);
                let r = rms(&asr);
                if let Some(d) = &dump {
                    let spec = hound::WavSpec {
                        channels: 1,
                        sample_rate: 16_000,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    };
                    let path = std::path::Path::new(d).join(format!("{name}.wav"));
                    let mut w = hound::WavWriter::create(path, spec).unwrap();
                    for s in asr.iter() {
                        w.write_sample((s * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
                            .unwrap();
                    }
                    w.finalize().unwrap();
                }
                let d = if r < ENGINE_SILENCE_RMS {
                    "blank_asr"
                } else {
                    "asr"
                };
                (d, sp.speech_ms(), r)
            }
        };
        println!(
            "{name}\t{:.1}\t{decision}\t{speech_ms}\t{:.1}\t{span}",
            rms(&pcm) * 32768.0,
            asr_rms * 32768.0
        );
    }
}
