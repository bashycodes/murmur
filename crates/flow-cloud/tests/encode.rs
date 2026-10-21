//! Upload encodings: FLAC is lossless (bit-exact round trip) and smaller
//! than WAV for speech-like audio.

use flow_cloud::encode::{encode, to_i16, AudioFormat};

fn speechy(ms: usize) -> Vec<f32> {
    // Two tones with a slow envelope plus a little noise.
    let mut seed = 12345u32;
    (0..ms * 16)
        .map(|i| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let n = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.01;
            let t = i as f32 / 16_000.0;
            let env = 0.5 + 0.5 * (t * 3.0).sin();
            env * (0.2 * (t * 220.0 * std::f32::consts::TAU).sin()
                + 0.1 * (t * 1330.0 * std::f32::consts::TAU).sin())
                + n
        })
        .collect()
}

#[test]
fn flac_round_trip_is_bit_exact() {
    let pcm = speechy(2500);
    let enc = encode(&pcm, AudioFormat::Flac);
    assert_eq!(enc.audio_ms, 2500);
    let mut r = claxon::FlacReader::new(std::io::Cursor::new(&enc.bytes)).unwrap();
    let info = r.streaminfo();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (16_000, 1, 16)
    );
    let back: Vec<i16> = r.samples().map(|s| s.unwrap() as i16).collect();
    assert_eq!(back, to_i16(&pcm));
}

#[test]
fn flac_is_smaller_than_wav() {
    let pcm = speechy(10_000);
    let flac = encode(&pcm, AudioFormat::Flac).bytes.len();
    let wav = encode(&pcm, AudioFormat::Wav).bytes.len();
    assert_eq!(wav, 44 + 10 * 16_000 * 2);
    assert!(flac < wav * 3 / 4, "flac {flac} vs wav {wav}");
}

#[test]
fn raw_pcm_is_little_endian_without_header() {
    let enc = encode(&[0.5, -0.5], AudioFormat::Pcm16);
    assert_eq!(enc.bytes, [0x00, 0x40, 0x00, 0xc0]);
}
