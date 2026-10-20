//! Hook-liveness watchdog (pure).
//!
//! Windows silently unhooks a `WH_KEYBOARD_LL` callback that overruns
//! `LowLevelHooksTimeout` (HKCU\Control Panel\Desktop; Win7+ removes the
//! hook with no notification), after which every shortcut just stops working.
//! Detection: when the system saw user input (`GetLastInputInfo` advanced)
//! but our hook saw no event since the last check, inject a tagged probe
//! key that the hook swallows. Probe not seen within `probe_timeout_ms`
//! → the hook is gone → re-install it.
//!
//! `GetLastInputInfo` also counts mouse input, so mouse-only activity
//! triggers probes too — rate-limited by `min_probe_gap_ms`, and only ever
//! while the user is active (an idle machine is never sent input, so screen
//! savers / sleep are unaffected).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub probe_timeout_ms: u64,
    pub min_probe_gap_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            probe_timeout_ms: 1_000,
            min_probe_gap_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Inject the probe, then call [`Watchdog::probe_sent`] if it went out.
    Probe,
    /// Hook is dead: unhook + re-install (and release held keys).
    Reinstall,
}

#[derive(Debug, Clone)]
pub struct Watchdog {
    cfg: Config,
    last_input: Option<u32>,
    last_events: u64,
    /// (sent at, hook event count at send).
    probe: Option<(u64, u64)>,
    last_probe_at: Option<u64>,
}

impl Watchdog {
    pub fn new(cfg: Config) -> Self {
        Watchdog {
            cfg,
            last_input: None,
            last_events: 0,
            probe: None,
            last_probe_at: None,
        }
    }

    /// Periodic check. `last_input_tick` = `GetLastInputInfo().dwTime`,
    /// `hook_events` = monotonic count of hook callbacks (any event).
    pub fn tick(&mut self, now_ms: u64, last_input_tick: u32, hook_events: u64) -> Action {
        let input_changed = self.last_input.is_some_and(|t| t != last_input_tick);
        let hook_quiet = hook_events == self.last_events;
        self.last_input = Some(last_input_tick);
        self.last_events = hook_events;

        if let Some((sent, events_at_send)) = self.probe {
            if hook_events > events_at_send {
                self.probe = None;
                return Action::Nothing;
            }
            if now_ms.saturating_sub(sent) >= self.cfg.probe_timeout_ms {
                self.probe = None;
                return Action::Reinstall;
            }
            return Action::Nothing;
        }
        let gap_ok = self
            .last_probe_at
            .is_none_or(|t| now_ms.saturating_sub(t) >= self.cfg.min_probe_gap_ms);
        if input_changed && hook_quiet && gap_ok {
            Action::Probe
        } else {
            Action::Nothing
        }
    }

    pub fn probe_sent(&mut self, now_ms: u64, hook_events: u64) {
        self.probe = Some((now_ms, hook_events));
        self.last_probe_at = Some(now_ms);
    }

    /// After a re-install: start fresh.
    pub fn reset(&mut self, hook_events: u64) {
        self.probe = None;
        self.last_events = hook_events;
        self.last_input = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wd() -> Watchdog {
        Watchdog::new(Config::default())
    }

    #[test]
    fn healthy_hook_with_typing_never_probes() {
        let mut w = wd();
        assert_eq!(w.tick(0, 100, 0), Action::Nothing);
        assert_eq!(w.tick(1000, 900, 12), Action::Nothing);
        assert_eq!(w.tick(2000, 1900, 30), Action::Nothing);
    }

    #[test]
    fn idle_machine_never_probes() {
        let mut w = wd();
        for t in 0..10 {
            assert_eq!(w.tick(t * 1000, 100, 5), Action::Nothing);
        }
    }

    #[test]
    fn input_without_hook_events_probes_then_reinstalls_if_unseen() {
        let mut w = wd();
        w.tick(0, 100, 5);
        assert_eq!(w.tick(1000, 900, 5), Action::Probe);
        w.probe_sent(1000, 5);
        assert_eq!(w.tick(1500, 1000, 5), Action::Nothing, "still waiting");
        assert_eq!(w.tick(2000, 1000, 5), Action::Reinstall);
    }

    #[test]
    fn seen_probe_means_alive_and_probes_are_rate_limited() {
        let mut w = wd();
        w.tick(0, 100, 5);
        assert_eq!(w.tick(1000, 900, 5), Action::Probe);
        w.probe_sent(1000, 5);
        assert_eq!(w.tick(2000, 1000, 6), Action::Nothing, "probe seen");
        // Mouse-only activity again: no probe before the 5 s gap.
        assert_eq!(w.tick(3000, 2900, 6), Action::Nothing);
        assert_eq!(w.tick(6000, 5900, 6), Action::Probe);
    }

    #[test]
    fn unsent_probe_is_not_awaited() {
        let mut w = wd();
        w.tick(0, 100, 5);
        assert_eq!(w.tick(1000, 900, 5), Action::Probe);
        // SendInput failed (secure desktop): caller doesn't call probe_sent.
        assert_eq!(w.tick(2000, 900, 5), Action::Nothing);
    }

    #[test]
    fn reset_after_reinstall() {
        let mut w = wd();
        w.tick(0, 100, 5);
        w.tick(1000, 900, 5);
        w.probe_sent(1000, 5);
        assert_eq!(w.tick(2000, 900, 5), Action::Reinstall);
        w.reset(5);
        assert_eq!(
            w.tick(3000, 950, 5),
            Action::Nothing,
            "first tick re-baselines"
        );
    }
}
