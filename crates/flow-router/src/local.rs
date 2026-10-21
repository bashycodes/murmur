//! `impl Backend for FlowCore`: the in-process engine.

use std::time::Instant;

use flow_core::config::{FieldContext, SessionConfig};
use flow_core::{CommitResult, FlowCore};

use crate::{now_unix_ms, Backend, BackendError, EngineKind, Failure, HealthReport, StartInfo};

impl Backend for FlowCore {
    fn kind(&self) -> EngineKind {
        EngineKind::Local
    }

    /// A `FlowCore` exists only with its models loaded.
    fn ready(&self) -> bool {
        true
    }

    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        FlowCore::start_session(self, cfg.clone())
            .map(|_| StartInfo::default())
            .map_err(|e| BackendError::new(Failure::Error, e.to_string()))
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        FlowCore::push_audio(self, pcm)
    }

    fn set_context(&self, ctx: &FieldContext) {
        FlowCore::set_context(self, ctx.clone())
    }

    fn partial(&self) -> Option<String> {
        FlowCore::partial(self)
    }

    /// In-process commit cannot be abandoned half way; the deadline is the
    /// remote backend's concern.
    fn commit(&self, _deadline: Instant) -> Result<CommitResult, BackendError> {
        Ok(FlowCore::commit(self))
    }

    fn cancel(&self) {
        FlowCore::cancel(self)
    }

    fn probe(&self) -> HealthReport {
        HealthReport {
            ok: true,
            engine: EngineKind::Local,
            postprocess_rev: Some(flow_core::POSTPROCESS_REV.into()),
            postprocess_match: Some(true),
            checked_at_ms: now_unix_ms(),
            ..Default::default()
        }
    }
}
