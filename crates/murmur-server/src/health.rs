//! `GET /v1/health` (same auth as the stream) → `hello_ok` JSON.

use std::sync::Arc;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Extension;

use crate::AppState;

pub(crate) fn hello_ok(state: &AppState, proto: u32) -> murmur_protocol::HelloOk {
    murmur_protocol::HelloOk {
        proto,
        // D26: product version + commit, so a client's probe pins the build.
        server: format!("murmur-server {}", murmur_build_info::FULL),
        core: murmur_core::VERSION.into(),
        postprocess_rev: murmur_core::POSTPROCESS_REV.into(),
        asr_backend: state.pool.asr_backend.clone(),
        asr_model: state.pool.asr_model.clone(),
        llm: state.pool.llm,
        workers: state.pool.workers(),
        busy: state.pool.busy(),
        max_samples: state.cfg.max_session_samples() as u64,
        features: std::iter::once(murmur_protocol::FEATURE_SEGMENTS)
            // D36: text-only cleanup needs a loaded LLM.
            .chain(state.pool.llm.then_some(murmur_protocol::FEATURE_CLEAN))
            // D53: meeting summary tasks ride on `clean`.
            .chain(state.pool.llm.then_some(murmur_protocol::FEATURE_SUMMARY))
            // D90: and the translate task.
            .chain(state.pool.llm.then_some(murmur_protocol::FEATURE_TRANSLATE))
            .map(String::from)
            .collect(),
        languages: state.pool.languages.keys().cloned().collect(),
        models: Box::new(state.models.clone()),
    }
}

pub(crate) async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    if let Err(d) = crate::authorize(&state, peer.ip(), &headers) {
        return d.into_response();
    }
    let body = serde_json::to_string(&hello_ok(&state, murmur_protocol::PROTO_MAX)).unwrap();
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
