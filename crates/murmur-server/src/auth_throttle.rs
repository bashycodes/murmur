//! Failed-auth throttle (D112): the bearer key is the only gate (D111),
//! so a peer IP that keeps sending wrong keys is refused for a while.
//!
//! [`AuthThrottle::check`] runs before the key comparison; the caller then
//! reports the outcome with [`AuthThrottle::failed`] or
//! [`AuthThrottle::succeeded`]. `max_failures` failures from one IP within
//! [`FAILURE_WINDOW`] lock it for `lockout_secs`; each further lockout
//! doubles that, up to [`MAX_LOCKOUT`], until the IP stays out of trouble
//! for [`STREAK_RESET`]. A success forgets the IP. Loopback is exempt.
//! An IPv6 peer counts as its /64 (one host usually holds a whole /64);
//! an IPv4-mapped IPv6 address counts as its IPv4. At most `max_tracked`
//! buckets are remembered: expired ones go first, then the least recently
//! seen (unlocked before locked).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Failures count towards a lockout only within this window.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(60);
/// The longest lockout, however often an IP repeats.
pub const MAX_LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// A lockout streak (the doubling) resets after this long without one.
pub const STREAK_RESET: Duration = Duration::from_secs(15 * 60);
/// Peer IPs remembered at once.
pub const MAX_TRACKED: usize = 4096;

/// `[auth_throttle]` in `config.toml`; every field is optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthThrottleConfig {
    /// Failed auths from one IP within 60 s that lock it; 0 = no throttle.
    pub max_failures: u32,
    /// The first lockout, in seconds; doubled on each repeat (≤ 15 min).
    pub lockout_secs: u64,
    /// Loopback peers are never throttled. Tests turn this off, since
    /// their client is on loopback; not a config file setting.
    #[serde(skip)]
    #[doc(hidden)]
    pub exempt_loopback: bool,
}

impl Default for AuthThrottleConfig {
    fn default() -> Self {
        Self {
            max_failures: 10,
            lockout_secs: 30,
            exempt_loopback: true,
        }
    }
}

/// Where "now" comes from (a manual clock in tests).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug)]
struct Peer {
    /// Failures in the current window, and when it started.
    failures: u32,
    window_start: Instant,
    /// Refused until then.
    locked_until: Option<Instant>,
    /// Lockouts in the current streak, and when the last one ended.
    streak: u32,
    last_lock_end: Option<Instant>,
    last_seen: Instant,
}

impl Peer {
    fn new(now: Instant) -> Self {
        Self {
            failures: 0,
            window_start: now,
            locked_until: None,
            streak: 0,
            last_lock_end: None,
            last_seen: now,
        }
    }

    fn locked(&self, now: Instant) -> bool {
        self.locked_until.is_some_and(|t| t > now)
    }

    /// Nothing left to remember: not locked, no live failures, no streak.
    fn expired(&self, now: Instant) -> bool {
        !self.locked(now)
            && (self.failures == 0 || now >= self.window_start + FAILURE_WINDOW)
            && self.last_lock_end.is_none_or(|t| now >= t + STREAK_RESET)
    }
}

pub struct AuthThrottle {
    cfg: AuthThrottleConfig,
    max_tracked: usize,
    clock: Box<dyn Clock>,
    peers: Mutex<HashMap<IpAddr, Peer>>,
}

impl AuthThrottle {
    pub fn new(cfg: AuthThrottleConfig) -> Self {
        Self::with_clock(cfg, Box::new(SystemClock), MAX_TRACKED)
    }

    pub fn with_clock(cfg: AuthThrottleConfig, clock: Box<dyn Clock>, max_tracked: usize) -> Self {
        Self {
            cfg,
            max_tracked: max_tracked.max(1),
            clock,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// The bucket `ip` counts in, or `None` when it is exempt.
    fn bucket(&self, ip: IpAddr) -> Option<IpAddr> {
        let ip = ip.to_canonical();
        if self.cfg.max_failures == 0 || (self.cfg.exempt_loopback && ip.is_loopback()) {
            return None;
        }
        Some(match ip {
            IpAddr::V6(v6) => IpAddr::V6((u128::from(v6) & !0u128 << 64).into()),
            v4 => v4,
        })
    }

    fn peers(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Peer>> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Err(retry_after)` while `ip` is locked out. Call before comparing
    /// the key.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let Some(ip) = self.bucket(ip) else {
            return Ok(());
        };
        let now = self.clock.now();
        match self.peers().get(&ip).and_then(|p| p.locked_until) {
            Some(until) if until > now => Err(until - now),
            _ => Ok(()),
        }
    }

    /// A wrong or missing key from `ip`. Returns the lockout this failure
    /// started, if any.
    pub fn failed(&self, ip: IpAddr) -> Option<Duration> {
        let peer = ip;
        let ip = self.bucket(ip)?;
        let now = self.clock.now();
        let mut peers = self.peers();
        if !peers.contains_key(&ip) && peers.len() >= self.max_tracked {
            evict(&mut peers, now);
        }
        let p = peers.entry(ip).or_insert_with(|| Peer::new(now));
        p.last_seen = now;
        if p.locked(now) {
            return None;
        }
        if now >= p.window_start + FAILURE_WINDOW {
            p.failures = 0;
            p.window_start = now;
        }
        p.failures += 1;
        if p.failures < self.cfg.max_failures {
            return None;
        }
        if p.last_lock_end.is_some_and(|t| now >= t + STREAK_RESET) {
            p.streak = 0;
        }
        let base = Duration::from_secs(self.cfg.lockout_secs.max(1));
        let window = base
            .checked_mul(1u32.checked_shl(p.streak).unwrap_or(u32::MAX))
            .unwrap_or(MAX_LOCKOUT)
            .min(MAX_LOCKOUT);
        let failures = p.failures;
        p.locked_until = Some(now + window);
        p.last_lock_end = p.locked_until;
        p.streak = p.streak.saturating_add(1);
        p.failures = 0;
        p.window_start = now;
        tracing::warn!(
            %peer,
            failures,
            lockout_s = window.as_secs(),
            "auth: too many failed attempts, peer locked out"
        );
        Some(window)
    }

    /// A valid key from `ip`: forget its failures and lockout streak.
    pub fn succeeded(&self, ip: IpAddr) {
        let Some(ip) = self.bucket(ip) else {
            return;
        };
        self.peers().remove(&ip);
    }

    /// Peer IPs currently remembered.
    pub fn tracked(&self) -> usize {
        self.peers().len()
    }
}

/// Make room for one more peer: drop every expired one; if none, the
/// least recently seen, preferring one that is not locked.
fn evict(peers: &mut HashMap<IpAddr, Peer>, now: Instant) {
    let before = peers.len();
    peers.retain(|_, p| !p.expired(now));
    if peers.len() < before {
        return;
    }
    if let Some(ip) = peers
        .iter()
        .min_by_key(|(_, p)| (p.locked(now), p.last_seen))
        .map(|(ip, _)| *ip)
    {
        peers.remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone)]
    struct Manual(Arc<Mutex<Instant>>);

    impl Manual {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }
        fn advance(&self, d: Duration) {
            *self.0.lock().unwrap() += d;
        }
    }

    impl Clock for Manual {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    fn cfg(max_failures: u32, lockout_secs: u64) -> AuthThrottleConfig {
        AuthThrottleConfig {
            max_failures,
            lockout_secs,
            ..Default::default()
        }
    }

    fn throttle(c: AuthThrottleConfig, max_tracked: usize) -> (AuthThrottle, Manual) {
        let clock = Manual::new();
        (
            AuthThrottle::with_clock(c, Box::new(clock.clone()), max_tracked),
            clock,
        )
    }

    const PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));

    fn ip(n: u32) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::from(0xC633_6400 + n)) // 198.51.100.0/24…
    }

    /// Fail until locked; returns the lockout window.
    fn lock(t: &AuthThrottle, ip: IpAddr, n: u32) -> Duration {
        for _ in 1..n {
            assert_eq!(t.failed(ip), None);
        }
        t.failed(ip).expect("the n-th failure locks")
    }

    #[test]
    fn locks_after_n_failures_within_the_window() {
        let (t, _) = throttle(cfg(10, 30), MAX_TRACKED);
        for _ in 0..9 {
            assert_eq!(t.failed(PEER), None);
            assert!(t.check(PEER).is_ok());
        }
        assert_eq!(t.failed(PEER), Some(Duration::from_secs(30)));
        assert_eq!(t.check(PEER), Err(Duration::from_secs(30)));
        // Another IP is unaffected.
        assert!(t.check(ip(1)).is_ok());
    }

    #[test]
    fn failures_older_than_the_window_do_not_count() {
        let (t, clock) = throttle(cfg(3, 30), MAX_TRACKED);
        t.failed(PEER);
        t.failed(PEER);
        clock.advance(FAILURE_WINDOW);
        assert_eq!(t.failed(PEER), None, "a new window starts at 1");
        assert_eq!(t.failed(PEER), None);
        assert!(t.failed(PEER).is_some());
    }

    #[test]
    fn recovers_after_the_window() {
        let (t, clock) = throttle(cfg(3, 30), MAX_TRACKED);
        lock(&t, PEER, 3);
        clock.advance(Duration::from_secs(20));
        assert_eq!(t.check(PEER), Err(Duration::from_secs(10)));
        // Failures while locked don't extend the lockout.
        assert_eq!(t.failed(PEER), None);
        clock.advance(Duration::from_secs(10));
        assert!(t.check(PEER).is_ok());
        // A fresh count after the lockout.
        assert_eq!(t.failed(PEER), None);
        assert!(t.check(PEER).is_ok());
    }

    #[test]
    fn backoff_doubles_up_to_the_cap_and_resets_after_a_quiet_period() {
        let (t, clock) = throttle(cfg(2, 30), MAX_TRACKED);
        let mut seen = Vec::new();
        for _ in 0..8 {
            let w = lock(&t, PEER, 2);
            seen.push(w.as_secs());
            clock.advance(w);
        }
        assert_eq!(seen, [30, 60, 120, 240, 480, 900, 900, 900]);
        clock.advance(STREAK_RESET);
        assert_eq!(lock(&t, PEER, 2), Duration::from_secs(30));
    }

    #[test]
    fn success_clears_failures_and_streak() {
        let (t, clock) = throttle(cfg(3, 30), MAX_TRACKED);
        t.failed(PEER);
        t.failed(PEER);
        t.succeeded(PEER);
        assert_eq!(t.failed(PEER), None, "count restarts after a success");
        assert_eq!(t.failed(PEER), None);
        let w = t.failed(PEER).unwrap();
        clock.advance(w);
        t.succeeded(PEER);
        assert_eq!(lock(&t, PEER, 3), Duration::from_secs(30), "streak reset");
        assert_eq!(t.tracked(), 1);
        clock.advance(Duration::from_secs(30));
        t.succeeded(PEER);
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn loopback_is_exempt() {
        let (t, _) = throttle(cfg(1, 30), MAX_TRACKED);
        for lo in ["127.0.0.1", "127.3.2.1", "::1", "::ffff:127.0.0.1"] {
            let lo: IpAddr = lo.parse().unwrap();
            assert_eq!(t.failed(lo), None);
            assert!(t.check(lo).is_ok());
        }
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn ipv4_mapped_v6_counts_as_its_ipv4() {
        let (t, _) = throttle(cfg(2, 30), MAX_TRACKED);
        t.failed("::ffff:203.0.113.7".parse().unwrap());
        assert!(t.failed(PEER).is_some());
        assert!(t.check("::ffff:203.0.113.7".parse().unwrap()).is_err());
    }

    #[test]
    fn ipv6_peers_count_per_64() {
        let (t, _) = throttle(cfg(2, 30), MAX_TRACKED);
        let a: IpAddr = "2001:db8:1:2::10".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:1:2:3".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::10".parse().unwrap();
        assert_eq!(t.failed(a), None);
        assert!(t.failed(b).is_some(), "same /64, same bucket");
        assert!(t.check(a).is_err());
        assert!(t.check(b).is_err());
        assert!(t.check(other).is_ok(), "another /64 is unaffected");
        assert_eq!(t.failed(other), None);
        assert_eq!(t.tracked(), 2);
        t.succeeded(b);
        assert!(t.check(a).is_ok(), "a success clears the whole /64");
    }

    #[test]
    fn zero_max_failures_turns_it_off() {
        let (t, _) = throttle(cfg(0, 30), MAX_TRACKED);
        for _ in 0..100 {
            assert_eq!(t.failed(PEER), None);
        }
        assert!(t.check(PEER).is_ok());
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn tracked_ips_are_capped() {
        let (t, clock) = throttle(cfg(2, 30), 4);
        // A spray of one failure each never grows past the cap.
        for n in 0..100 {
            t.failed(ip(n));
            assert!(t.tracked() <= 4);
        }
        // Expired entries go first: after the window, a locked IP survives
        // a fresh spray because the stale ones are dropped instead.
        clock.advance(FAILURE_WINDOW);
        lock(&t, PEER, 2);
        for n in 200..203 {
            t.failed(ip(n));
        }
        assert_eq!(t.tracked(), 4);
        // Full of live entries: the oldest unlocked one goes, not PEER.
        t.failed(ip(300));
        assert_eq!(t.tracked(), 4);
        assert!(t.check(PEER).is_err(), "the locked IP is kept");
    }
}
