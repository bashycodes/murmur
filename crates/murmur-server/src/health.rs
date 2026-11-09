//! `GET /v1/health` (same auth as the stream) → `hello_ok` JSON.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::AppState;

pub(crate) fn hello_ok(state: &AppState, proto: u32) -> flow_protocol::HelloOk {
    flow_protocol::HelloOk {
        proto,
        // D26: product version + commit, so a client's probe pins the build.
        server: format!("flow-server {}", flow_build_info::FULL),
        core: flow_core::VERSION.into(),
        postprocess_rev: flow_core::POSTPROCESS_REV.into(),
        asr_backend: state.pool.asr_backend.clone(),
        asr_model: state.pool.asr_model.clone(),
        llm: state.pool.llm,
        workers: state.pool.workers(),
        busy: state.pool.busy(),
        max_samples: state.cfg.max_session_samples() as u64,
        features: std::iter::once(flow_protocol::FEATURE_SEGMENTS)
            // D36: text-only cleanup needs a loaded LLM.
            .chain(state.pool.llm.then_some(flow_protocol::FEATURE_CLEAN))
            // D53: meeting summary tasks ride on `clean`.
            .chain(state.pool.llm.then_some(flow_protocol::FEATURE_SUMMARY))
            .map(String::from)
            .collect(),
        languages: state.pool.languages.keys().cloned().collect(),
    }
}

pub(crate) async fn handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if crate::authorize(&state, &headers).is_err() {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let body = serde_json::to_string(&hello_ok(&state, flow_protocol::PROTO_MAX)).unwrap();
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
