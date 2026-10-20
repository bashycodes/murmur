//! Cue sounds. Synthesised at startup (no bundled third-party WAVs, no
//! files to ship) and played with `PlaySoundW(SND_MEMORY|SND_ASYNC)`.

use crate::dictation::Sound;

pub const RATE: u32 = 22_050;

/// One tone segment: frequency sweep `f0 → f1` over `ms`.
#[derive(Debug, Clone, Copy)]
struct Seg {
    f0: f32,
    f1: f32,
    ms: u32,
}

fn recipe(s: Sound) -> (&'static [Seg], f32) {
    const START: &[Seg] = &[
        Seg {
            f0: 660.0,
            f1: 660.0,
            ms: 60,
        },
        Seg {
            f0: 880.0,
            f1: 880.0,
            ms: 80,
        },
    ];
    const STOP: &[Seg] = &[
        Seg {
            f0: 880.0,
            f1: 880.0,
            ms: 60,
        },
        Seg {
            f0: 660.0,
            f1: 660.0,
            ms: 80,
        },
    ];
    const LOCK: &[Seg] = &[
        Seg {
            f0: 660.0,
            f1: 660.0,
            ms: 50,
        },
        Seg {
            f0: 880.0,
            f1: 880.0,
            ms: 50,
        },
        Seg {
            f0: 1320.0,
            f1: 1320.0,
            ms: 80,
        },
    ];
    const PASTE: &[Seg] = &[Seg {
        f0: 1400.0,
        f1: 1000.0,
        ms: 45,
    }];
    const ERROR: &[Seg] = &[
        Seg {
            f0: 300.0,
            f1: 220.0,
            ms: 140,
        },
        Seg {
            f0: 220.0,
            f1: 180.0,
            ms: 160,
        },
    ];
    match s {
        Sound::Start => (START, 0.30),
        Sound::Stop => (STOP, 0.30),
        Sound::Lock => (LOCK, 0.30),
        Sound::Paste => (PASTE, 0.20),
        Sound::Error => (ERROR, 0.35),
    }
}

/// Mono PCM16 samples for a cue (each segment gets a 5 ms raised-cosine
/// fade in/out so there are no clicks).
pub fn samples(s: Sound) -> Vec<i16> {
    let (segs, amp) = recipe(s);
    let mut out = Vec::new();
    let mut phase = 0.0f32;
    for seg in segs {
        let n = (RATE * seg.ms / 1000) as usize;
        let fade = ((RATE as usize * 5) / 1000).min(n / 2).max(1);
        for i in 0..n {
            let t = i as f32 / n as f32;
            let f = seg.f0 + (seg.f1 - seg.f0) * t;
            phase += 2.0 * std::f32::consts::PI * f / RATE as f32;
            let env = if i < fade {
                0.5 - 0.5 * (std::f32::consts::PI * i as f32 / fade as f32).cos()
            } else if i >= n - fade {
                let k = (n - 1 - i) as f32;
                0.5 - 0.5 * (std::f32::consts::PI * k / fade as f32).cos()
            } else {
                1.0
            };
            out.push((phase.sin() * amp * env * 32767.0) as i16);
        }
    }
    out
}

/// RIFF/WAVE PCM16 mono container.
pub fn wav_bytes(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
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
    for s in pcm {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

pub fn wav(s: Sound) -> Vec<u8> {
    wav_bytes(&samples(s), RATE)
}

#[cfg(windows)]
pub mod win {
    use super::wav;
    use crate::dictation::Sound;
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    static WAVS: OnceLock<[Vec<u8>; 5]> = OnceLock::new();

    fn idx(s: Sound) -> usize {
        match s {
            Sound::Start => 0,
            Sound::Stop => 1,
            Sound::Lock => 2,
            Sound::Paste => 3,
            Sound::Error => 4,
        }
    }

    /// Fire-and-forget. Buffers are 'static, as SND_ASYNC|SND_MEMORY requires.
    pub fn play(s: Sound) {
        let w = WAVS.get_or_init(|| {
            [
                wav(Sound::Start),
                wav(Sound::Stop),
                wav(Sound::Lock),
                wav(Sound::Paste),
                wav(Sound::Error),
            ]
        });
        let buf = &w[idx(s)];
        unsafe {
            let _ = PlaySoundW(
                PCWSTR(buf.as_ptr() as *const u16),
                None,
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Sound; 5] = [
        Sound::Start,
        Sound::Stop,
        Sound::Lock,
        Sound::Paste,
        Sound::Error,
    ];

    #[test]
    fn wav_header_is_consistent() {
        let w = wav(Sound::Start);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let riff = u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, w.len());
        let data = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(data + 44, w.len());
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), RATE);
    }

    #[test]
    fn durations_match_recipes() {
        assert_eq!(
            samples(Sound::Start).len(),
            (RATE * 60 / 1000 + RATE * 80 / 1000) as usize
        );
        assert_eq!(samples(Sound::Paste).len(), (RATE * 45 / 1000) as usize);
    }

    #[test]
    fn cues_are_short_bounded_and_click_free() {
        for s in ALL {
            let p = samples(s);
            assert!(
                !p.is_empty() && p.len() < RATE as usize / 2,
                "{s:?} < 500 ms"
            );
            let peak = p.iter().map(|x| x.unsigned_abs()).max().unwrap();
            assert!(peak <= (0.35 * 32767.0) as u16 + 1, "{s:?} peak {peak}");
            assert!(p[0].unsigned_abs() < 200, "{s:?} starts silent");
            assert!(p.last().unwrap().unsigned_abs() < 200, "{s:?} ends silent");
        }
    }

    #[test]
    fn cues_are_distinct() {
        assert_ne!(samples(Sound::Start), samples(Sound::Stop));
        assert_ne!(samples(Sound::Start), samples(Sound::Lock));
    }
}
