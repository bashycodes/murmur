//! Server cleanup hop (D36): the configured flow-server's LLM as one
//! [`Cleaner`] of the cleanup chain, through the remote backend's text-only
//! `clean` request. No network code here: the backend does the talking.
//!
//! Refuses fast ([`CleanError::Unavailable`]) when server cleanup is off or
//! not configured, its circuit is open, the server is known to have no LLM,
//! or the prompt carries no [`CleanPrompt::source`] to send, so the chain
//! moves on at once.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::AbortFlag;
use flow_core::cleanup::{CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope};

use crate::router::{Circuit, LockExt};
use crate::{Backend, CircuitStatus, Failure};

pub(crate) struct ServerCleanState {
    /// The remote backend of the current server config; `None` = server
    /// cleanup is not wanted (not in the order, no server, no remote
    /// support in this build).
    pub backend: Option<Arc<dyn Backend>>,
    pub circuit: Circuit,
    /// Bumped when the server config changes: results of calls started
    /// before do not touch the new circuit.
    pub epoch: u64,
}

pub(crate) struct ServerClean {
    pub st: Mutex<ServerCleanState>,
}

fn unavailable(why: &str) -> CleanError {
    CleanError::Unavailable(why.into())
}

impl ServerClean {
    pub fn new() -> Self {
        Self {
            st: Mutex::new(ServerCleanState {
                backend: None,
                circuit: Circuit::new(false),
                epoch: 0,
            }),
        }
    }

    pub fn backend(&self) -> Option<Arc<dyn Backend>> {
        self.st.lk().backend.clone()
    }

    /// Would a call be tried now? `hello` unknown counts as yes (the call
    /// connects and finds out).
    pub fn ready(&self) -> bool {
        let st = self.st.lk();
        let Some(b) = st.backend.as_ref() else {
            return false;
        };
        !st.circuit.is_open(Instant::now()) && self::server_llm(b.as_ref()) != Some(false)
    }

    /// `Some(has clean)` once the backend has seen a `hello_ok`.
    pub fn server_llm(&self) -> Option<bool> {
        let b = self.backend()?;
        server_llm(b.as_ref())
    }

    pub fn circuit_status(&self) -> Option<CircuitStatus> {
        let st = self.st.lk();
        st.backend
            .as_ref()
            .map(|_| st.circuit.status(Instant::now()))
    }

    pub fn reset_circuit(&self) {
        self.st.lk().circuit = Circuit::new(false);
    }

    fn settle(&self, epoch: u64, f: Option<Failure>) {
        let mut st = self.st.lk();
        if st.epoch != epoch {
            return;
        }
        match f {
            None => {
                if !st.circuit.sticky {
                    st.circuit = Circuit::new(false);
                }
            }
            Some(f) => st.circuit.record_failure(f, Instant::now()),
        }
    }
}

fn server_llm(b: &dyn Backend) -> Option<bool> {
    b.server_hello()
        .map(|h| h.features.iter().any(|f| f == flow_protocol::FEATURE_CLEAN))
}

impl Cleaner for ServerClean {
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
        _cancel: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let none = CleanUsage::default();
        let Some(src) = prompt.source.as_ref() else {
            return (Err(unavailable("no_source")), none);
        };
        let (backend, epoch) = {
            let st = self.st.lk();
            let Some(b) = st.backend.clone() else {
                return (
                    Err(unavailable(Failure::ServerNotConfigured.as_str())),
                    none,
                );
            };
            if st.circuit.is_open(Instant::now()) {
                return (Err(unavailable(Failure::CircuitOpen.as_str())), none);
            }
            (b, st.epoch)
        };
        if server_llm(backend.as_ref()) == Some(false) {
            return (Err(unavailable("server_no_llm")), none);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return (Err(CleanError::Backend("timeout".into())), none);
        }
        let mut req = (**src).clone();
        req.timeout_ms = (left.as_millis() as u64).max(1);
        let usage = |model: Option<String>| CleanUsage {
            engine: Some("server".into()),
            model,
            ..Default::default()
        };
        match backend.clean_text(&req, deadline) {
            Ok(c) => {
                let u = usage(c.model.clone());
                match (c.text, c.error) {
                    (Some(t), _) => {
                        self.settle(epoch, None);
                        (Ok(t), u)
                    }
                    (None, e) => {
                        let e = e.unwrap_or_else(|| "server_error".into());
                        // The server answered: the connection is fine.
                        self.settle(epoch, None);
                        match e.as_str() {
                            "no_llm" => (Err(unavailable("server_no_llm")), none),
                            "busy" | "too_large" => (Err(unavailable(&e)), none),
                            "llm_timeout" => (Err(CleanError::Backend("timeout".into())), u),
                            _ => (Err(CleanError::Backend(e)), u),
                        }
                    }
                }
            }
            Err(e) => {
                self.settle(epoch, Some(e.kind));
                let err = match e.kind {
                    Failure::Timeout | Failure::Disconnected => {
                        CleanError::Backend(e.kind.as_str().into())
                    }
                    k => unavailable(k.as_str()),
                };
                let u = match err {
                    CleanError::Backend(_) => usage(None),
                    CleanError::Unavailable(_) => none,
                };
                (Err(err), u)
            }
        }
    }

    /// The user's own machine: it may see everything the on-device LLM does.
    fn prompt_scope(&self) -> PromptScope {
        PromptScope::FULL
    }

    fn engine(&self) -> &'static str {
        "server"
    }

    /// Known to have no LLM (or switched off): the core skips the step.
    fn available(&self) -> bool {
        self.backend()
            .is_some_and(|b| server_llm(b.as_ref()) != Some(false))
    }

    fn model(&self) -> Option<String> {
        None
    }
}
