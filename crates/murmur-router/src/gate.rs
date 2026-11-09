//! Cloud cleanup gate (D28 §2.5/§2.6, D52): wraps the injected cleaner of
//! the selected provider (OpenRouter or the user's custom server) with the
//! per-call checks the router owns. No network code.
//!
//! Every call re-checks C1 for cleanup — build support, `allow_cloud_cleanup`
//! and `cloud` in `cleanup_order`, cleanup consent for the provider's host,
//! a key (OpenRouter; optional for a custom server),
//! the cleanup circuit and, for OpenRouter only, the monthly budget
//! (catalogue estimate before the call; D52: the custom server is the
//! owner's own and never budget-gated) — and refuses *fast* (`CleanError::Unavailable`) so a
//! `RoutedCleaner` can use the on-device LLM or the deterministic text.
//! After the call it records the exact `usage.cost` (or the estimate, when
//! a generation exists but reported no cost) and feeds the circuit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use murmur_core::asr::AbortFlag;
use murmur_core::cleanup::{CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope};

use crate::cloud::{
    month_of, CloudConfig, CloudProvider, CloudUsage, SecretString, Usd, PURPOSE_CLEANUP,
};
use crate::router::{failure_from_str, Circuit, LockExt};
use crate::{now_unix_ms, CircuitStatus, Failure};

/// What the router last told the gate.
pub(crate) struct GateState {
    /// Build support + toggle + order.
    pub wanted: bool,
    /// The cloud config (D52: provider, OpenRouter or custom server entry).
    pub cfg: CloudConfig,
    pub budget: Option<Usd>,
    pub key: Option<SecretString>,
    pub circuit: Circuit,
    /// The provider's cleaner for (cleanup view, key).
    pub inner: Option<Arc<dyn Cleaner>>,
    /// Real cost of the last call per (provider, model) (estimate for
    /// unpriced models, spec §2.6 rule 5).
    last_cost: HashMap<(CloudProvider, String), f64>,
    pub last: Option<CleanUsage>,
    /// Bumped when the cleanup config or key changes: results of calls
    /// started before do not touch the new circuit.
    pub epoch: u64,
}

pub(crate) struct Gate {
    pub st: Mutex<GateState>,
    pub usage: Arc<Mutex<CloudUsage>>,
}

/// The cleaner to call, the pre-flight estimate, the gate epoch.
type Admitted = (Arc<dyn Cleaner>, Option<f64>, u64);

fn unavailable(f: Failure) -> CleanError {
    CleanError::Unavailable(f.as_str().into())
}

impl Gate {
    pub fn new(usage: Arc<Mutex<CloudUsage>>) -> Self {
        Self {
            st: Mutex::new(GateState {
                wanted: false,
                cfg: CloudConfig::default(),
                budget: None,
                key: None,
                circuit: Circuit::new(true),
                inner: None,
                last_cost: HashMap::new(),
                last: None,
                epoch: 0,
            }),
            usage,
        }
    }

    /// Why a call now would be refused, without the budget (status page).
    pub fn not_ready(&self) -> Option<Failure> {
        let st = self.st.lk();
        Self::check_static(&st).err()
    }

    fn check_static(st: &GateState) -> Result<Arc<dyn Cleaner>, Failure> {
        let needs_key = st.cfg.cleanup_needs_key();
        if !st.wanted
            || !st.cfg.cleanup_ready()
            || !st.cfg.cleanup_consent_ok()
            || (needs_key && st.key.as_ref().is_none_or(|k| k.is_empty()))
        {
            return Err(Failure::CloudCleanupNotConfigured);
        }
        let inner = st.inner.clone().ok_or(Failure::CloudCleanupNotConfigured)?;
        if st.circuit.is_open(Instant::now()) {
            return Err(if st.circuit.sticky {
                st.circuit.reason.unwrap_or(Failure::CircuitOpen)
            } else {
                Failure::CircuitOpen
            });
        }
        Ok(inner)
    }

    pub fn circuit_status(&self) -> CircuitStatus {
        self.st.lk().circuit.status(Instant::now())
    }

    pub fn reset_circuit(&self) {
        self.st.lk().circuit = Circuit::new(true);
    }

    pub fn last(&self) -> Option<CleanUsage> {
        self.st.lk().last.clone()
    }

    fn cost_key(cfg: &CloudConfig) -> (CloudProvider, String) {
        (cfg.provider, cfg.cleanup_model().to_string())
    }

    fn spent(&self) -> f64 {
        let mut u = self.usage.lk();
        u.roll(&month_of(now_unix_ms()));
        u.budget_cost()
    }

    /// Pre-flight: the cleaner to call, the estimate, the epoch.
    fn admit(&self, prompt: &CleanPrompt) -> Result<Admitted, CleanError> {
        let st = self.st.lk();
        let inner = Self::check_static(&st).map_err(unavailable)?;
        let est = inner
            .estimate_cost_usd(prompt)
            .or_else(|| st.last_cost.get(&Self::cost_key(&st.cfg)).map(|c| c * 2.0));
        // D52: the budget is OpenRouter's; the custom server is not gated.
        let budget = st.budget.filter(|_| st.cfg.cleanup_budgeted());
        if let Some(b) = budget {
            let spent = self.spent();
            match est {
                Some(e) if spent + e > b.0 => return Err(unavailable(Failure::BudgetExceeded)),
                Some(_) => {}
                // Unknown OpenRouter price with a budget: allowed only once
                // a real cost of this model was recorded (spec §2.6 rule 5).
                None => return Err(unavailable(Failure::BudgetExceeded)),
            }
        }
        Ok((inner, est, st.epoch))
    }

    fn settle(
        &self,
        epoch: u64,
        est: Option<f64>,
        r: &Result<String, CleanError>,
        u: &mut CleanUsage,
    ) {
        let mut st = self.st.lk();
        // Money spent is real whatever the epoch.
        let (cost, estimated) = match (u.cost_usd, est) {
            (Some(c), _) => (Some(c.0), u.cost_estimated),
            // A generation exists but no usage came back: estimate it.
            (None, Some(e)) if u.generation_id.is_some() || r.is_ok() => (Some(e), true),
            // No generation: OpenRouter bills nothing.
            _ => (None, false),
        };
        let reached = !matches!(r, Err(CleanError::Unavailable(_)));
        if reached || cost.is_some() {
            let mut usage = self.usage.lk();
            usage.roll(&month_of(now_unix_ms()));
            usage.record_for(
                st.cfg.cleanup_key_id(),
                PURPOSE_CLEANUP,
                0,
                Some(Usd(cost.unwrap_or(0.0))),
                estimated,
            );
        }
        if let (Some(c), false) = (cost, estimated) {
            let k = Self::cost_key(&st.cfg);
            st.last_cost.insert(k, c);
        }
        u.cost_usd = cost.map(Usd);
        u.cost_estimated = estimated;
        if epoch == st.epoch {
            let now = Instant::now();
            match r {
                Ok(_) => {
                    if !st.circuit.sticky {
                        st.circuit = Circuit::new(true);
                    }
                }
                Err(e) => {
                    if let Some(f) = failure_from_str(e.code()) {
                        st.circuit.record_failure(f, now);
                    }
                }
            }
        }
        st.last = Some(u.clone());
    }
}

impl Cleaner for Gate {
    fn clean(&self, prompt: &CleanPrompt, deadline: Instant) -> Result<String, CleanError> {
        self.clean_metered(prompt, deadline, &AbortFlag::default())
            .0
    }

    fn clean_cancellable(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> Result<String, CleanError> {
        self.clean_metered(prompt, deadline, cancel).0
    }

    fn clean_metered(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let mut base = CleanUsage {
            engine: Some("cloud".into()),
            model: self.model(),
            ..Default::default()
        };
        let (inner, est, epoch) = match self.admit(prompt) {
            Ok(x) => x,
            Err(e) => return (Err(e), base),
        };
        let (r, mut u) = inner.clean_metered(prompt, deadline, cancel);
        u.engine = base.engine.take();
        if u.model.is_none() {
            u.model = base.model.take();
        }
        self.settle(epoch, est, &r, &mut u);
        (r, u)
    }

    /// Nothing with content is sent (murmur-cloud warms its connection);
    /// still only when a call would be admitted.
    fn prefill(&self, prompt: &CleanPrompt) {
        let inner = Self::check_static(&self.st.lk()).ok();
        if let Some(i) = inner {
            i.prefill(prompt);
        }
    }

    fn info_json(&self) -> Option<String> {
        let inner = self.st.lk().inner.clone();
        inner.and_then(|i| i.info_json())
    }

    fn prompt_scope(&self) -> PromptScope {
        self.st.lk().cfg.cleanup_scope()
    }

    fn engine(&self) -> &'static str {
        "cloud"
    }

    fn model(&self) -> Option<String> {
        Some(self.st.lk().cfg.cleanup_model().to_string())
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(self.st.lk().cfg.cleanup_timeout_ms())
    }

    fn estimate_cost_usd(&self, prompt: &CleanPrompt) -> Option<f64> {
        let inner = self.st.lk().inner.clone();
        inner.and_then(|i| i.estimate_cost_usd(prompt))
    }
}
