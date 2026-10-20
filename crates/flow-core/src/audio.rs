//! Audio helpers for shells/dev tools: downmix + resample to 16 kHz PCM16.
//! Real shells capture 16 kHz mono directly (WASAPI / AudioRecord).

use crate::config::SAMPLE_RATE;

pub fn downmix(interleaved: &[i16], channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|f| (f.iter().map(|&s| s as i32).sum::<i32>() / channels as i32) as i16)
        .collect()
}

/// Linear-interpolation resampler; adequate for speech into Whisper.
pub fn resample_to_16k(pcm: &[i16], rate: u32) -> Vec<i16> {
    if rate as usize == SAMPLE_RATE || pcm.is_empty() {
        return pcm.to_vec();
    }
    let out_len = (pcm.len() as u64 * SAMPLE_RATE as u64 / rate as u64) as usize;
    let step = rate as f64 / SAMPLE_RATE as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let j = pos as usize;
            let f = pos - j as f64;
            let a = pcm[j] as f64;
            let b = *pcm.get(j + 1).unwrap_or(&pcm[j]) as f64;
            (a + (b - a) * f).round() as i16
        })
        .collect()
}

/// Read any PCM WAV (int or float) as 16 kHz mono PCM16.
#[cfg(feature = "wav")]
pub fn read_wav_16k_mono(path: &std::path::Path) -> Result<Vec<i16>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let raw: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample.saturating_sub(16);
            r.samples::<i32>()
                .map(|s| s.map(|v| (v >> shift) as i16))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
        hound::SampleFormat::Float => r
            .samples::<f32>()
            .map(|s| s.map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
    };
    Ok(resample_to_16k(
        &downmix(&raw, spec.channels as usize),
        spec.sample_rate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[100, 300, -50, 50], 2), vec![200, 0]);
        assert_eq!(downmix(&[1, 2, 3], 1), vec![1, 2, 3]);
    }

    #[test]
    fn resample_halves_length_from_32k() {
        let x: Vec<i16> = (0..3200).map(|i| (i % 100) as i16).collect();
        assert_eq!(resample_to_16k(&x, 32_000).len(), 1600);
    }

    #[test]
    fn resample_identity_at_16k() {
        let x = vec![1i16, 2, 3];
        assert_eq!(resample_to_16k(&x, 16_000), x);
    }

    #[test]
    fn resample_44k1_length() {
        let x = vec![0i16; 44_100];
        assert_eq!(resample_to_16k(&x, 44_100).len(), 16_000);
    }
}
