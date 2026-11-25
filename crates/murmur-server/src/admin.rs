//! Admin API v1 (D39, `murmur_protocol::admin`): list, create and revoke
//! named keys over HTTP JSON, authenticated with the same bearer key as
//! the stream. `GET /v1/whoami` answers any active key; `/v1/admin/...`
//! needs an admin key (else 403). The new key's secret is in the create
//! answer only: never logged, never stored (the key store keeps its hash).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Extension, Router};
use murmur_protocol::admin::{
    code, AdminError, CreateKey, CreatedKey, KeyList, KeyView, RevokedKey, WhoAmI,
    MAX_ADMIN_BODY_BYTES, PATH_ADMIN_KEYS, PATH_WHOAMI,
};
use serde::Serialize;

use crate::keys::{KeyError, KeyInfo, KeyRef, KeyStore};
use crate::AppState;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(PATH_WHOAMI, get(whoami))
        .route(PATH_ADMIN_KEYS, get(list).post(create))
        .route(&format!("{PATH_ADMIN_KEYS}/{{id}}"), delete(revoke))
        .layer(DefaultBodyLimit::max(MAX_ADMIN_BODY_BYTES))
}

fn json<T: Serialize>(status: StatusCode, v: &T) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            // The create answer carries a secret; nothing here is cacheable.
            (header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(v).expect("admin body serialises"),
    )
        .into_response()
}

fn error(status: StatusCode, c: &str, message: impl Into<String>) -> Response {
    json(status, &AdminError::new(c, message))
}

fn key_error(e: KeyError) -> Response {
    let (status, c) = match &e {
        KeyError::BadName(_) => (StatusCode::BAD_REQUEST, code::BAD_REQUEST),
        KeyError::NameTaken(_) => (StatusCode::CONFLICT, code::NAME_TAKEN),
        KeyError::NotFound(_) => (StatusCode::NOT_FOUND, code::NOT_FOUND),
        KeyError::LastAdmin(_) => (StatusCode::CONFLICT, code::LAST_ADMIN),
        KeyError::Io(m) => {
            tracing::warn!("admin: keys file: {m}");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                code::SERVER_ERROR,
                "the server could not write its keys file",
            );
        }
    };
    error(status, c, e.to_string())
}

/// The caller's key and the store, or the refusal: 404 `auth_off`, 429
/// `throttled` (D112), 401, or (with `need_admin`) 403.
fn caller(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    need_admin: bool,
) -> Result<(KeyRef, Arc<KeyStore>), Box<Response>> {
    let Some(store) = state.keys.clone() else {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            code::AUTH_OFF,
            "this server runs without keys (auth = \"none\")",
        )));
    };
    let key = match crate::authorize(state, peer.ip(), headers) {
        Ok(Some(key)) => key,
        Ok(None) | Err(crate::Denied::Unauthorized) => {
            return Err(Box::new(error(
                StatusCode::UNAUTHORIZED,
                code::UNAUTHORIZED,
                "unknown or revoked key",
            )))
        }
        Err(crate::Denied::Throttled(d)) => {
            let mut r = error(
                StatusCode::TOO_MANY_REQUESTS,
                code::THROTTLED,
                "too many failed attempts from this address; try again later",
            );
            r.headers_mut().insert(
                header::RETRY_AFTER,
                crate::Denied::retry_after_secs(d).into(),
            );
            return Err(Box::new(r));
        }
    };
    if need_admin && !key.admin {
        tracing::info!(key = %key.id, "admin: refused, not an admin key");
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            code::FORBIDDEN,
            "this key is not an admin key",
        )));
    }
    Ok((key, store))
}

fn view(k: &KeyInfo, current: &str) -> KeyView {
    KeyView {
        id: k.key.id.clone(),
        name: k.key.name.clone(),
        admin: k.key.admin,
        created: k.key.created,
        revoked: k.key.revoked,
        last_used: k.usage.last_used,
        last_connected: k.usage.last_connected,
        last_seen: k.usage.last_connected,
        client: k.usage.client.clone(),
        dictations: k.usage.dictations,
        audio_ms: k.usage.audio_ms,
        downloads: k.usage.downloads,
        download_bytes: k.usage.download_bytes,
        current: k.key.id == current,
    }
}

/// The listed view of key `id` (just created or revoked).
fn view_of(store: &KeyStore, id: &str, current: &str) -> KeyView {
    store
        .list()
        .iter()
        .find(|k| k.key.id == id)
        .map(|k| view(k, current))
        .unwrap_or_default()
}

async fn whoami(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    match caller(&state, peer, &headers, false) {
        Ok((k, _)) => json(
            StatusCode::OK,
            &WhoAmI {
                id: k.id,
                name: k.name,
                admin: k.admin,
            },
        ),
        Err(r) => *r,
    }
}

async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    let (me, store) = match caller(&state, peer, &headers, true) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let keys = tokio::task::spawn_blocking(move || store.list())
        .await
        .unwrap_or_default();
    json(
        StatusCode::OK,
        &KeyList {
            keys: keys.iter().map(|k| view(k, &me.id)).collect(),
        },
    )
}

async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (me, store) = match caller(&state, peer, &headers, true) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let req: CreateKey = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, code::BAD_REQUEST, e.to_string()),
    };
    let me_id = me.id.clone();
    let r = tokio::task::spawn_blocking(move || {
        let (k, secret) = store.add(&req.name, req.admin)?;
        Ok::<_, KeyError>((view_of(&store, &k.id, &me_id), secret))
    })
    .await;
    match r {
        Ok(Ok((key, secret))) => {
            tracing::info!(by = %me.id, key = %key.id, name = %key.name, admin = key.admin, "admin: key added");
            json(StatusCode::CREATED, &CreatedKey { key, secret })
        }
        Ok(Err(e)) => key_error(e),
        Err(e) => key_error(KeyError::Io(e.to_string())),
    }
}

async fn revoke(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (me, store) = match caller(&state, peer, &headers, true) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let me_id = me.id.clone();
    let r = tokio::task::spawn_blocking(move || {
        // The API never forces: the last admin key stays (CLI --force can).
        let k = store.revoke_id(&id, false)?;
        Ok::<_, KeyError>(view_of(&store, &k.id, &me_id))
    })
    .await;
    match r {
        Ok(Ok(key)) => {
            tracing::info!(by = %me.id, key = %key.id, name = %key.name, "admin: key revoked");
            json(StatusCode::OK, &RevokedKey { key })
        }
        Ok(Err(e)) => key_error(e),
        Err(e) => key_error(KeyError::Io(e.to_string())),
    }
}
