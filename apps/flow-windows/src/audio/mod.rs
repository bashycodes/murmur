//! Audio: device-rate interleaved f32 → 16 kHz mono i16 for `push_audio`,
//! plus an RMS level for the overlay bars.

#[cfg(windows)]
pub mod capture;

pub const TARGET_RATE: u32 = 16_000;

/// Streaming downmix + resample to 16 kHz mono i16 (linear interpolation,
/// boxcar pre-filter when decimating). State carries across chunks so
/// chunked and one-shot processing produce identical output.
#[derive(Debug, Clone)]
pub struct Resampler {
    channels: usize,
    step: f64,
    t: f64,
    prev: Option<f32>,
    box_len: usize,
    box_buf: Vec<f32>,
    box_idx: usize,
    box_sum: f32,
    box_filled: usize,
}

impl Resampler {
    pub fn new(in_rate: u32, channels: u16) -> Self {
        let step = in_rate.max(1) as f64 / TARGET_RATE as f64;
        let box_len = if step > 1.0 { step.round() as usize } else { 1 };
        Resampler {
            channels: channels.max(1) as usize,
            step,
            t: 0.0,
            prev: None,
            box_len,
            box_buf: vec![0.0; box_len],
            box_idx: 0,
            box_sum: 0.0,
            box_filled: 0,
        }
    }

    fn prefilter(&mut self, x: f32) -> f32 {
        if self.box_len == 1 {
            return x;
        }
        self.box_sum += x - self.box_buf[self.box_idx];
        self.box_buf[self.box_idx] = x;
        self.box_idx = (self.box_idx + 1) % self.box_len;
        self.box_filled = (self.box_filled + 1).min(self.box_len);
        self.box_sum / self.box_filled as f32
    }

    pub fn process(&mut self, interleaved: &[f32]) -> Vec<i16> {
        let mut out = Vec::with_capacity((interleaved.len() as f64 / self.step) as usize + 2);
        for frame in interleaved.chunks_exact(self.channels) {
            let mono = frame.iter().sum::<f32>() / self.channels as f32;
            let x = self.prefilter(mono);
            match self.prev {
                None => {
                    out.push(to_i16(x));
                    self.t = self.step;
                }
                Some(p) => {
                    while self.t <= 1.0 {
                        out.push(to_i16(p + (x - p) * self.t as f32));
                        self.t += self.step;
                    }
                    self.t -= 1.0;
                }
            }
            self.prev = Some(x);
        }
        out
    }
}

fn to_i16(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

/// RMS of a chunk, normalised to 0..1 of full scale.
pub fn rms(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm
        .iter()
        .map(|s| {
            let f = *s as f64 / 32768.0;
            f * f
        })
        .sum();
    (sum / pcm.len() as f64).sqrt() as f32
}

/// Map RMS to a perceptual 0..1 bar scale (−60 dBFS → 0, 0 dBFS → 1).
pub fn level_from_rms(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Accumulates 16 kHz audio and emits one overlay level per `window`
/// samples (default ≈30 Hz), whatever chunk sizes the device delivers.
#[derive(Debug, Clone)]
pub struct LevelMeter {
    window: usize,
    sum_sq: f64,
    n: usize,
}

impl LevelMeter {
    pub fn new(window: usize) -> Self {
        LevelMeter {
            window: window.max(1),
            sum_sq: 0.0,
            n: 0,
        }
    }

    /// Returns the levels of all windows completed by this chunk.
    pub fn push(&mut self, pcm: &[i16]) -> Vec<f32> {
        let mut out = Vec::new();
        for s in pcm {
            let f = *s as f64 / 32768.0;
            self.sum_sq += f * f;
            self.n += 1;
            if self.n == self.window {
                let r = (self.sum_sq / self.n as f64).sqrt() as f32;
                out.push(level_from_rms(r));
                self.sum_sq = 0.0;
                self.n = 0;
            }
        }
        out
    }
}

impl Default for LevelMeter {
    fn default() -> Self {
        LevelMeter::new(TARGET_RATE as usize / 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_meter_emits_per_window_across_chunks() {
        let mut m = LevelMeter::new(100);
        assert!(m.push(&[16384; 60]).is_empty());
        let l = m.push(&[16384; 140]);
        assert_eq!(l.len(), 2);
        assert!((l[0] - level_from_rms(0.5)).abs() < 1e-4);
        assert_eq!(m.push(&[0; 99]).len(), 0);
        assert_eq!(m.push(&[0; 1]), vec![0.0]);
        assert_eq!(LevelMeter::default().window, 533);
    }

    fn sine(rate: u32, freq: f32, secs: f32, channels: u16) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        let mut v = Vec::with_capacity(n * channels as usize);
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5;
            for _ in 0..channels {
                v.push(s);
            }
        }
        v
    }

    fn zero_crossings(x: &[i16]) -> usize {
        x.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count()
    }

    #[test]
    fn passthrough_at_16k_mono() {
        let mut r = Resampler::new(16_000, 1);
        let out = r.process(&[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(out, vec![0, 16384, -16384, 32767]);
    }

    #[test]
    fn decimates_48k_stereo_to_16k_length() {
        let mut r = Resampler::new(48_000, 2);
        let out = r.process(&sine(48_000, 440.0, 1.0, 2));
        assert!((out.len() as i64 - 16_000).abs() <= 1, "{}", out.len());
    }

    #[test]
    fn preserves_frequency_44k1() {
        let mut r = Resampler::new(44_100, 1);
        let out = r.process(&sine(44_100, 440.0, 1.0, 1));
        let zc = zero_crossings(&out) as i64;
        assert!((zc - 880).abs() <= 4, "zc={zc}");
        assert!((out.len() as i64 - 16_000).abs() <= 1);
    }

    #[test]
    fn upsamples_8k() {
        let mut r = Resampler::new(8_000, 1);
        let out = r.process(&sine(8_000, 300.0, 1.0, 1));
        assert!((out.len() as i64 - 16_000).abs() <= 2);
    }

    #[test]
    fn chunked_equals_one_shot() {
        let input = sine(48_000, 1000.0, 0.5, 2);
        let mut a = Resampler::new(48_000, 2);
        let one = a.process(&input);
        let mut b = Resampler::new(48_000, 2);
        let mut chunked = Vec::new();
        for c in input.chunks(2 * 37) {
            chunked.extend(b.process(c));
        }
        assert_eq!(one, chunked);
    }

    #[test]
    fn stereo_downmix_averages_channels() {
        let mut r = Resampler::new(16_000, 2);
        let out = r.process(&[1.0, 0.0, 0.5, 0.5]);
        assert_eq!(out, vec![16384, 16384]);
    }

    #[test]
    fn clamps_out_of_range() {
        let mut r = Resampler::new(16_000, 1);
        assert_eq!(r.process(&[2.0, -2.0]), vec![32767, -32768]);
    }

    #[test]
    fn dc_survives_decimation_filter() {
        let mut r = Resampler::new(48_000, 1);
        let out = r.process(&vec![0.25; 4800]);
        assert!(out[10..].iter().all(|s| (*s - 8192).abs() <= 1));
    }

    #[test]
    fn rms_values() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0; 100]), 0.0);
        assert!((rms(&[32767, -32767]) - 1.0).abs() < 1e-3);
        assert!((rms(&[16384, -16384]) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn level_mapping() {
        assert_eq!(level_from_rms(0.0), 0.0);
        assert!((level_from_rms(1.0) - 1.0).abs() < 1e-6);
        assert!((level_from_rms(0.031_622_8) - 0.5).abs() < 1e-3, "-30 dBFS");
        assert_eq!(level_from_rms(1e-6), 0.0);
    }
}
