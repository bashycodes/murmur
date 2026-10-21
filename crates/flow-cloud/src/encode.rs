//! Audio encoding for upload (docs/cloud-stt.md §6.3) and chunking at
//! quiet points (§6.2). Input is the core's 16 kHz mono f32.

use flow_core::config::SAMPLE_RATE;

/// Wire format of an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// FLAC, 16 kHz mono 16-bit (pure-Rust `flacenc`).
    Flac,
    /// RIFF WAV PCM16.
    Wav,
    /// Raw signed 16-bit little-endian PCM, 16 kHz mono, no container.
    Pcm16,
}

impl AudioFormat {
    pub fn mime(self) -> &'static str {
        match self {
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Pcm16 => "application/octet-stream",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
            AudioFormat::Pcm16 => "pcm",
        }
    }
}

/// One encoded upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedAudio {
    pub format: AudioFormat,
    pub bytes: Vec<u8>,
    pub audio_ms: u32,
}

pub fn to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect()
}

pub fn encode(pcm: &[f32], format: AudioFormat) -> EncodedAudio {
    let s = to_i16(pcm);
    let bytes = match format {
        AudioFormat::Flac => flac(&s),
        AudioFormat::Wav => wav(&s),
        AudioFormat::Pcm16 => raw(&s),
    };
    EncodedAudio {
        format,
        bytes,
        audio_ms: (pcm.len() as u64 * 1000 / SAMPLE_RATE as u64) as u32,
    }
}

pub fn raw(s: &[i16]) -> Vec<u8> {
    s.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn wav(s: &[i16]) -> Vec<u8> {
    let data_len = (s.len() * 2) as u32;
    let rate = SAMPLE_RATE as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v.extend_from_slice(&raw(s));
    v
}

pub fn flac(s: &[i16]) -> Vec<u8> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let samples: Vec<i32> = s.iter().map(|&v| v as i32).collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .expect("default flacenc config is valid");
    let source = flacenc::source::MemSource::from_samples(&samples, 1, 16, SAMPLE_RATE);
    match flacenc::encode_with_fixed_block_size(&config, source, config.block_size) {
        Ok(stream) => {
            let mut sink = flacenc::bitsink::ByteSink::new();
            match stream.write(&mut sink) {
                Ok(()) => sink.as_slice().to_vec(),
                Err(_) => wav(s),
            }
        }
        // Never expected for 16-bit mono; WAV is always accepted.
        Err(_) => wav(s),
    }
}

/// Split `pcm` into pieces of at most `max_samples`, cutting each piece at
/// the quietest 300 ms stretch within the last 10 s before the limit (a
/// pause between words), else at the limit itself.
pub fn chunk_ranges(pcm: &[f32], max_samples: usize) -> Vec<std::ops::Range<usize>> {
    const FRAME: usize = SAMPLE_RATE / 50; // 20 ms
    const RUN: usize = 15; // 300 ms
    let max_samples = max_samples.max(SAMPLE_RATE);
    let mut out = Vec::new();
    let mut start = 0;
    while pcm.len() - start > max_samples {
        let limit = start + max_samples;
        let lo = limit
            .saturating_sub(SAMPLE_RATE * 10)
            .max(start + SAMPLE_RATE / 2);
        let frames: Vec<f32> = (lo..limit)
            .step_by(FRAME)
            .map(|f| {
                let e = (f + FRAME).min(limit);
                pcm[f..e].iter().map(|s| s * s).sum::<f32>() / (e - f).max(1) as f32
            })
            .collect();
        let cut = if frames.len() >= RUN {
            let mut best = (f32::INFINITY, 0usize);
            let mut sum: f32 = frames[..RUN].iter().sum();
            for i in 0..=frames.len() - RUN {
                if i > 0 {
                    sum += frames[i + RUN - 1] - frames[i - 1];
                }
                // Prefer the latest of equally quiet runs (bigger chunks).
                if sum <= best.0 {
                    best = (sum, i);
                }
            }
            lo + (best.1 + RUN / 2) * FRAME
        } else {
            limit
        };
        let cut = cut.clamp(start + 1, limit);
        out.push(start..cut);
        start = cut;
    }
    out.push(start..pcm.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: usize, amp: f32) -> Vec<f32> {
        (0..ms * 16)
            .map(|i| (i as f32 * 0.07).sin() * amp)
            .collect()
    }

    #[test]
    fn wav_header_is_canonical() {
        let w = wav(&[1, -2, 3]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(&w[44..], &[1, 0, 0xfe, 0xff, 3, 0]);
    }

    #[test]
    fn to_i16_clamps() {
        assert_eq!(
            to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]),
            [0, 32767, -32767, 32767, -32767]
        );
    }

    #[test]
    fn short_audio_is_one_chunk() {
        let pcm = tone(3000, 0.3);
        assert_eq!(chunk_ranges(&pcm, 16_000 * 55), vec![0..pcm.len()]);
    }

    #[test]
    fn long_audio_is_cut_at_the_pause() {
        // 48 s speech, 1 s pause, 40 s speech; limit 55 s.
        let mut pcm = tone(48_000, 0.3);
        let pause_at = pcm.len();
        pcm.extend(vec![0.0; 16_000]);
        pcm.extend(tone(40_000, 0.3));
        let r = chunk_ranges(&pcm, 16_000 * 55);
        assert_eq!(r.len(), 2);
        assert!(
            (pause_at..pause_at + 16_000).contains(&r[0].end),
            "cut {} not in the pause",
            r[0].end
        );
        assert_eq!(r[1].end, pcm.len());
    }

    #[test]
    fn no_pause_cuts_at_or_before_the_limit_and_covers_everything() {
        let pcm = tone(130_000, 0.3);
        let r = chunk_ranges(&pcm, 16_000 * 55);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|c| c.len() <= 16_000 * 55));
        assert_eq!(r[0].start, 0);
        assert_eq!(r.last().unwrap().end, pcm.len());
        assert!(r.windows(2).all(|w| w[0].end == w[1].start));
    }
}
