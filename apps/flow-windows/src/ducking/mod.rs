//! Mute/duck other apps' audio while dictating (Wispr's `shouldMuteAudio`,
//! default on for Windows). Plan + restore rules are pure; `win` enumerates
//! WASAPI render sessions (`IAudioSessionManager2` → `ISimpleAudioVolume`).

#[cfg(windows)]
pub mod win;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionVol {
    /// Index/identity of the session within one enumeration.
    pub key: usize,
    pub pid: u32,
    pub volume: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckEntry {
    pub key: usize,
    pub original: f32,
    pub target: f32,
}

/// Sessions to change: never our own process, never the system-sounds
/// session (pid 0), skip already-silent ones. `factor` 0 = mute.
pub fn plan(sessions: &[SessionVol], own_pid: u32, factor: f32) -> Vec<DuckEntry> {
    sessions
        .iter()
        .filter(|s| s.pid != 0 && s.pid != own_pid && s.volume > 0.0)
        .map(|s| DuckEntry {
            key: s.key,
            original: s.volume,
            target: (s.volume * factor).clamp(0.0, 1.0),
        })
        .collect()
}

/// On restore: put the original back only if the volume is still what we
/// set — if the user moved the slider meanwhile, respect their choice.
pub fn restore_value(e: &DuckEntry, current: f32) -> Option<f32> {
    ((current - e.target).abs() < 1e-3).then_some(e.original)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(key: usize, pid: u32, volume: f32) -> SessionVol {
        SessionVol { key, pid, volume }
    }

    #[test]
    fn plan_skips_self_system_and_silent() {
        let p = plan(
            &[s(0, 0, 1.0), s(1, 42, 0.8), s(2, 7, 1.0), s(3, 9, 0.0)],
            42,
            0.0,
        );
        assert_eq!(
            p,
            vec![DuckEntry {
                key: 2,
                original: 1.0,
                target: 0.0
            }]
        );
    }

    #[test]
    fn plan_ducks_by_factor() {
        let p = plan(&[s(0, 5, 0.5)], 1, 0.2);
        assert!((p[0].target - 0.1).abs() < 1e-6);
    }

    #[test]
    fn restore_only_if_untouched() {
        let e = DuckEntry {
            key: 0,
            original: 0.7,
            target: 0.0,
        };
        assert_eq!(restore_value(&e, 0.0), Some(0.7));
        assert_eq!(restore_value(&e, 0.0005), Some(0.7), "float noise");
        assert_eq!(restore_value(&e, 0.4), None, "user changed it");
    }
}
