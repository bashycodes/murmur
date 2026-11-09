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
/// `rate == 0` (an unknown rate) gives no audio rather than a panic.
pub fn resample_to_16k(pcm: &[i16], rate: u32) -> Vec<i16> {
    if rate == 0 {
        return Vec::new();
    }
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

/// Streaming linear resampler to 16 kHz (the same interpolation as
/// [`resample_to_16k`], carried across blocks).
#[derive(Debug, Clone)]
pub struct Resampler {
    step: f64,
    /// Source position of the next output sample, relative to `prev`'s
    /// index (`prev` is source sample -1 of the next block).
    pos: f64,
    prev: Option<i16>,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        Self {
            step: rate.max(1) as f64 / SAMPLE_RATE as f64,
            pos: 0.0,
            prev: None,
        }
    }

    /// Resample the next block (mono).
    pub fn push(&mut self, pcm: &[i16], out: &mut Vec<i16>) {
        if (self.step - 1.0).abs() < f64::EPSILON {
            out.extend_from_slice(pcm);
            return;
        }
        // Source = [prev?] ++ pcm; `pos` indexes it.
        let off = usize::from(self.prev.is_some());
        let at = |i: usize| -> i16 {
            if i < off {
                self.prev.unwrap_or(0)
            } else {
                pcm[i - off]
            }
        };
        let n = pcm.len() + off;
        while n > 0 && (self.pos as usize) + 1 < n {
            let j = self.pos as usize;
            let f = self.pos - j as f64;
            let (a, b) = (at(j) as f64, at(j + 1) as f64);
            out.push((a + (b - a) * f).round() as i16);
            self.pos += self.step;
        }
        if n > 0 {
            // Keep the last sample as the next block's index 0.
            self.prev = Some(at(n - 1));
            self.pos -= (n - 1) as f64;
        }
    }
}

/// A PCM WAV (int or float, any rate and channel count) read block by
/// block as 16 kHz mono PCM16: memory stays bounded for hour-long files.
#[cfg(feature = "wav")]
pub struct WavBlocks {
    reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    spec: hound::WavSpec,
    resampler: Resampler,
    block: usize,
    done: bool,
}

#[cfg(feature = "wav")]
impl WavBlocks {
    /// `block_samples` = output samples per block (about).
    pub fn open(path: &std::path::Path, block_samples: usize) -> Result<Self, String> {
        let reader =
            hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let spec = reader.spec();
        Ok(Self {
            resampler: Resampler::new(spec.sample_rate),
            reader,
            spec,
            block: block_samples.max(1),
            done: false,
        })
    }

    /// Length of the whole file at 16 kHz, in samples.
    pub fn total_samples(&self) -> usize {
        (self.reader.duration() as u64 * SAMPLE_RATE as u64 / self.spec.sample_rate.max(1) as u64)
            as usize
    }
}

#[cfg(feature = "wav")]
impl Iterator for WavBlocks {
    type Item = Result<Vec<i16>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let ch = self.spec.channels.max(1) as usize;
        let src = self.block * self.spec.sample_rate.max(1) as usize / SAMPLE_RATE * ch;
        let mut raw = Vec::with_capacity(src.max(ch));
        let res: Result<(), hound::Error> = match self.spec.sample_format {
            hound::SampleFormat::Int => {
                let shift = self.spec.bits_per_sample.saturating_sub(16);
                self.reader
                    .samples::<i32>()
                    .take(src.max(ch))
                    .try_for_each(|s| s.map(|v| raw.push((v >> shift) as i16)))
            }
            hound::SampleFormat::Float => self
                .reader
                .samples::<f32>()
                .take(src.max(ch))
                .try_for_each(|s| s.map(|v| raw.push((v.clamp(-1.0, 1.0) * 32767.0) as i16))),
        };
        if let Err(e) = res {
            self.done = true;
            return Some(Err(e.to_string()));
        }
        if raw.is_empty() {
            self.done = true;
            return None;
        }
        let mut out = Vec::with_capacity(self.block + 2);
        self.resampler.push(&downmix(&raw, ch), &mut out);
        Some(Ok(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_resampler_matches_the_batch_one() {
        let x: Vec<i16> = (0..44_100)
            .map(|i| ((i * 37) % 2000) as i16 - 1000)
            .collect();
        for rate in [44_100u32, 48_000, 8_000, 22_050, 16_000] {
            let batch = resample_to_16k(&x, rate);
            for block in [1usize, 441, 1000, 44_100] {
                let mut r = Resampler::new(rate);
                let mut out = Vec::new();
                for c in x.chunks(block) {
                    r.push(c, &mut out);
                }
                let n = batch.len().min(out.len());
                // The stream holds back the output past the last source
                // sample (at most one source sample's worth).
                let slack = (16_000 / rate as usize).max(1);
                assert!(batch.len().abs_diff(out.len()) <= slack, "{rate} {block}");
                // Same interpolation; float rounding may differ by 1 LSB.
                let worst = (0..n).map(|i| (batch[i] - out[i]).abs()).max().unwrap_or(0);
                assert!(worst <= 1, "{rate} {block}: {worst}");
            }
        }
    }

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

    #[test]
    fn resample_from_rate_zero_is_empty_not_a_panic() {
        assert!(resample_to_16k(&[1i16, 2, 3], 0).is_empty());
    }
}
