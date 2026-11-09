//! Hands-free endpointing: notice that the user stopped talking.
//!
//! Without it, hands-free ends on a key press or the check button. Opt-in
//! per session (D19) with `SessionConfig.endpoint_silence_ms`.
//!
//! It is the commit-time VAD (`vad`, D16) run as a stream: the same 20 ms
//! frames, the same RMS/ZCR speech bars and the same 60 ms minimum run, but
//! the noise floor comes from the last 30 s of frames seen so far. The
//! endpoint is reached once at least [`MIN_SPEECH_MS`] of speech was heard
//! and the audio since the last speech run is at least the configured
//! silence. It latches: once reached it stays reached for the session, so a
//! shell polling it cannot miss it or see it flicker.
//!
//! The detector only reads the audio the shell pushes, so `MurmurCore` and
//! `murmur_router::Router` (every route: local, own server, cloud) compute
//! the same answer from the same pushes, with no protocol change.

use serde::{Deserialize, Serialize};

use crate::config::SAMPLE_RATE;
use crate::vad::{StreamVad, FRAME};

/// Recommended `endpoint_silence_ms` for a shell's hands-free mode: long
/// enough for a pause to think mid-sentence, short enough that waiting for
/// the paste does not feel stuck (D19). 2000 → 1500 with D22: with
/// pause-segmented decoding the endpoint wait is nearly all of hands-free
/// release→text, and 1200 starts to end dictations on thinking pauses.
pub const RECOMMENDED_SILENCE_MS: u32 = 1500;
/// Smaller configured values are raised to this.
pub const MIN_SILENCE_MS: u32 = 300;
/// Speech needed before the endpoint can fire, so a cough, a click or the
/// start chime does not end a dictation that has not begun.
pub const MIN_SPEECH_MS: u32 = 300;
const FRAME_MS: u64 = (FRAME * 1000 / SAMPLE_RATE) as u64;

/// Endpoint state of the current session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EndpointState {
    /// The user spoke and has been silent for `endpoint_silence_ms`.
    /// Latched for the rest of the session.
    pub reached: bool,
    /// Speech heard so far (20 ms frames in runs of ≥ 60 ms).
    pub speech_ms: u64,
    /// Audio since the end of the last speech run (since the start when
    /// there was no speech yet).
    pub trailing_silence_ms: u64,
}

/// Streaming endpoint detector over pushed PCM.
#[derive(Debug, Clone)]
pub struct Endpointer {
    silence_frames: u64,
    min_speech_frames: u64,
    vad: StreamVad,
    reached: bool,
}

impl Endpointer {
    /// `silence_ms` below [`MIN_SILENCE_MS`] is raised to it.
    pub fn new(silence_ms: u32) -> Self {
        let frames = |ms: u32| (ms.max(1) as u64).div_ceil(FRAME_MS);
        Self {
            silence_frames: frames(silence_ms.max(MIN_SILENCE_MS)),
            min_speech_frames: frames(MIN_SPEECH_MS),
            vad: StreamVad::new(),
            reached: false,
        }
    }

    /// The detector for a session config (`None` = endpointing off).
    pub fn for_session(cfg: &crate::config::SessionConfig) -> Option<Self> {
        cfg.endpoint_silence_ms.map(Self::new)
    }

    /// Feed PCM16 as pushed by the shell.
    pub fn push_i16(&mut self, pcm: &[i16]) {
        for &s in pcm {
            self.sample(s as f32 / 32768.0);
        }
    }

    /// Feed normalised samples (as the commit-time VAD sees them).
    pub fn push(&mut self, pcm: &[f32]) {
        for &s in pcm {
            self.sample(s);
        }
    }

    fn sample(&mut self, s: f32) {
        if self.vad.sample(s)
            && !self.reached
            && self.vad.speech_frames() >= self.min_speech_frames
            && self.vad.trailing_frames() >= self.silence_frames
        {
            self.reached = true;
        }
    }

    pub fn reached(&self) -> bool {
        self.reached
    }

    pub fn state(&self) -> EndpointState {
        EndpointState {
            reached: self.reached,
            speech_ms: self.vad.speech_frames() * FRAME_MS,
            trailing_silence_ms: self.vad.trailing_frames() * FRAME_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: usize = SAMPLE_RATE;

    fn voiced(ms: usize) -> Vec<i16> {
        (0..ms * S / 1000)
            .map(|i| {
                let t = i as f32 / S as f32;
                (6000.0 * (2.0 * std::f32::consts::PI * 150.0 * t).sin()) as i16
            })
            .collect()
    }

    fn noise(ms: usize, amp: f32, seed: u32) -> Vec<i16> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        (0..ms * S / 1000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                ((x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp * 32768.0) as i16
            })
            .collect()
    }

    /// Push `pcm` in 40 ms chunks (the clients' chunk size); returns the
    /// pushed time (ms) at which the endpoint was first reached.
    fn fire_at(ep: &mut Endpointer, pcm: &[i16], offset_ms: usize) -> Option<usize> {
        for (i, c) in pcm.chunks(S / 25).enumerate() {
            ep.push_i16(c);
            if ep.reached() {
                return Some(offset_ms + (i + 1) * 40);
            }
        }
        None
    }

    #[test]
    fn fires_after_configured_trailing_silence() {
        let mut ep = Endpointer::new(1000);
        assert_eq!(fire_at(&mut ep, &noise(500, 0.001, 1), 0), None);
        assert_eq!(fire_at(&mut ep, &voiced(1500), 500), None);
        let at = fire_at(&mut ep, &noise(3000, 0.001, 2), 2000).expect("endpoint");
        assert!((3000..=3080).contains(&at), "{at}");
        let st = ep.state();
        assert!(st.reached);
        assert!((1400..=1600).contains(&st.speech_ms), "{st:?}");
        assert!(st.trailing_silence_ms >= 1000, "{st:?}");
    }

    #[test]
    fn never_fires_without_speech() {
        let mut ep = Endpointer::new(500);
        assert_eq!(fire_at(&mut ep, &noise(10_000, 0.004, 3), 0), None);
        assert_eq!(fire_at(&mut ep, &vec![0; 5 * S], 0), None);
        assert_eq!(ep.state().speech_ms, 0);
    }

    #[test]
    fn a_short_blip_is_not_enough_speech() {
        let mut ep = Endpointer::new(500);
        let mut pcm = noise(1000, 0.001, 4);
        pcm.extend(voiced(100)); // < MIN_SPEECH_MS
        pcm.extend(noise(3000, 0.001, 5));
        assert_eq!(fire_at(&mut ep, &pcm, 0), None);
    }

    #[test]
    fn pauses_shorter_than_the_threshold_do_not_fire() {
        let mut ep = Endpointer::new(1500);
        let mut pcm = Vec::new();
        for i in 0..4 {
            pcm.extend(voiced(800));
            pcm.extend(noise(1200, 0.001, 10 + i));
        }
        assert_eq!(fire_at(&mut ep, &pcm, 0), None);
        assert!(fire_at(&mut ep, &noise(2000, 0.001, 20), 8000).is_some());
    }

    #[test]
    fn latches_once_reached() {
        let mut ep = Endpointer::new(500);
        let mut pcm = voiced(600);
        pcm.extend(vec![0; S]);
        assert!(fire_at(&mut ep, &pcm, 0).is_some());
        ep.push_i16(&voiced(1000));
        assert!(ep.reached());
    }

    #[test]
    fn chunking_does_not_change_the_answer() {
        let mut pcm = noise(300, 0.002, 6);
        pcm.extend(voiced(700));
        pcm.extend(noise(1700, 0.002, 7));
        let mut whole = Endpointer::new(1200);
        whole.push_i16(&pcm);
        let mut odd = Endpointer::new(1200);
        for c in pcm.chunks(333) {
            odd.push_i16(c);
        }
        assert_eq!(whole.state(), odd.state());
        assert!(whole.reached());
    }

    #[test]
    fn tiny_thresholds_are_raised_to_the_minimum() {
        let mut ep = Endpointer::new(0);
        let mut pcm = voiced(500);
        pcm.extend(vec![0; S / 5]); // 200 ms < MIN_SILENCE_MS
        assert_eq!(fire_at(&mut ep, &pcm, 0), None);
        assert!(fire_at(&mut ep, &vec![0; S / 5], 700).is_some());
    }
}
