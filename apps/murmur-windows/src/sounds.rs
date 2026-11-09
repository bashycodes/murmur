//! Cue sounds. Synthesised at startup (no bundled third-party WAVs, no
//! files to ship) and played with `PlaySoundW(SND_MEMORY|SND_ASYNC)`.
//!
//! A user can replace any cue with their own file,
//! `%APPDATA%\Murmur\sounds\<name>.wav` ([`file_name`]). The table is
//! built once, at the first cue, so new files need a restart. A missing
//! file uses the built-in tone; an unreadable, non-WAV or oversized one
//! does too, with one `[sounds]` line on stderr.

use crate::dictation::Sound;
use std::io::Read;
use std::path::{Path, PathBuf};

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

/// Largest custom cue file accepted (bigger → built-in tone).
pub const MAX_CUSTOM_BYTES: usize = 2 * 1024 * 1024;

/// File name of a cue's custom override in [`sounds_dir`].
pub fn file_name(s: Sound) -> &'static str {
    match s {
        Sound::Start => "start.wav",
        Sound::Stop => "stop.wav",
        Sound::Lock => "lock.wav",
        Sound::Paste => "paste.wav",
        Sound::Error => "error.wav",
    }
}

/// `%APPDATA%\Murmur\sounds`.
pub fn sounds_dir(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join("sounds")
}

/// Minimal check that `b` can be handed to `PlaySoundW(SND_MEMORY)`:
/// a RIFF/WAVE header and at most [`MAX_CUSTOM_BYTES`].
pub fn validate_custom(b: &[u8]) -> Result<(), String> {
    if b.len() > MAX_CUSTOM_BYTES {
        return Err(format!(
            "larger than {} MB",
            MAX_CUSTOM_BYTES / (1024 * 1024)
        ));
    }
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    Ok(())
}

/// The bytes to play for `s`, given the result of reading its override
/// file. Returns the built-in cue when the read failed or the file is
/// invalid, plus a log reason (`None` when the file is just absent, the
/// normal case, or when the override is used).
pub fn choose(s: Sound, read: std::io::Result<Vec<u8>>) -> (Vec<u8>, Option<String>) {
    match read {
        Ok(b) => match validate_custom(&b) {
            Ok(()) => (b, None),
            Err(why) => (wav(s), Some(why)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (wav(s), None),
        Err(e) => (wav(s), Some(format!("unreadable: {e}"))),
    }
}

/// Read at most `MAX_CUSTOM_BYTES + 1` bytes, so an oversized file is
/// detected without loading all of it.
fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_CUSTOM_BYTES as u64 + 1)
        .read_to_end(&mut v)?;
    Ok(v)
}

/// The cue for `s`: the user's file in `dir` if valid, else built-in.
/// Logs one `[sounds]` line when a present file is rejected.
pub fn load(s: Sound, dir: &Path) -> Vec<u8> {
    let path = dir.join(file_name(s));
    let (bytes, why) = choose(s, read_capped(&path));
    if let Some(why) = why {
        eprintln!(
            "[sounds] {}: {why}; using the built-in sound",
            path.display()
        );
    }
    bytes
}

#[cfg(windows)]
pub mod win {
    use super::{load, sounds_dir};
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
            let appdata = std::env::var("APPDATA").ok();
            let dir = sounds_dir(appdata.as_deref());
            [
                load(Sound::Start, &dir),
                load(Sound::Stop, &dir),
                load(Sound::Lock, &dir),
                load(Sound::Paste, &dir),
                load(Sound::Error, &dir),
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

    fn not_found() -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    #[test]
    fn valid_override_is_used() {
        let custom = wav_bytes(&[0, 100, -100, 0], 8000);
        let (b, why) = choose(Sound::Start, Ok(custom.clone()));
        assert_eq!(b, custom);
        assert!(why.is_none());
    }

    #[test]
    fn missing_override_uses_builtin_quietly() {
        let (b, why) = choose(Sound::Stop, not_found());
        assert_eq!(b, wav(Sound::Stop));
        assert!(why.is_none());
    }

    #[test]
    fn unreadable_override_uses_builtin_and_says_why() {
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let (b, why) = choose(Sound::Lock, Err(e));
        assert_eq!(b, wav(Sound::Lock));
        assert!(why.unwrap().contains("unreadable"));
    }

    #[test]
    fn non_riff_override_uses_builtin() {
        for junk in [
            b"ID3\x03not a wav file".to_vec(),
            Vec::new(),
            b"RIFF".to_vec(),
        ] {
            let (b, why) = choose(Sound::Paste, Ok(junk));
            assert_eq!(b, wav(Sound::Paste));
            assert!(why.unwrap().contains("RIFF/WAVE"));
        }
        // RIFF but not WAVE (e.g. an AVI).
        let mut avi = wav(Sound::Start);
        avi[8..12].copy_from_slice(b"AVI ");
        assert!(choose(Sound::Start, Ok(avi)).1.is_some());
    }

    #[test]
    fn oversize_override_uses_builtin() {
        let pcm = vec![0i16; MAX_CUSTOM_BYTES / 2];
        let big = wav_bytes(&pcm, RATE);
        assert!(big.len() > MAX_CUSTOM_BYTES);
        let (b, why) = choose(Sound::Error, Ok(big));
        assert_eq!(b, wav(Sound::Error));
        assert!(why.unwrap().contains("larger"));
        // Exactly at the limit is fine.
        let mut edge = wav_bytes(&[], RATE);
        edge.resize(MAX_CUSTOM_BYTES, 0);
        assert!(choose(Sound::Error, Ok(edge)).1.is_none());
    }

    #[test]
    fn load_reads_files_from_the_sounds_dir() {
        let dir = std::env::temp_dir().join(format!("murmur-sounds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let custom = wav_bytes(&[1, 2, 3], 8000);
        std::fs::write(dir.join("start.wav"), &custom).unwrap();
        std::fs::write(dir.join("stop.wav"), b"not a wav").unwrap();
        assert_eq!(load(Sound::Start, &dir), custom);
        assert_eq!(load(Sound::Stop, &dir), wav(Sound::Stop));
        assert_eq!(load(Sound::Lock, &dir), wav(Sound::Lock));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sounds_dir_and_names() {
        let d = sounds_dir(Some("AD"));
        assert!(d.ends_with(Path::new("Murmur").join("sounds")));
        let names: Vec<_> = ALL.iter().map(|s| file_name(*s)).collect();
        assert_eq!(
            names,
            [
                "start.wav",
                "stop.wav",
                "lock.wav",
                "paste.wav",
                "error.wav"
            ]
        );
    }

    #[test]
    fn cues_are_distinct() {
        assert_ne!(samples(Sound::Start), samples(Sound::Stop));
        assert_ne!(samples(Sound::Start), samples(Sound::Lock));
    }
}
