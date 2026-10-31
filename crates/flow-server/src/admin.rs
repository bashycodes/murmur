//! Admin API v1 (D39, `flow_protocol::admin`): list, create and revoke
//! named keys over HTTP JSON, authenticated with the same bearer key as
//! the stream. `GET /v1/whoami` answers any active key; `/v1/admin/...`
//! needs an admin key (else 403). The new key's secret is in the create
//! answer only: never logged, never stored (the key store keeps its hash).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::Router;
use flow_protocol::admin::{
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

/// Why a request may not use the admin API: status, code, message.
type Refusal = (StatusCode, &'static str, &'static str);

/// The caller's key and the store, or why not: 404 `auth_off`, 401, or
/// (with `need_admin`) 403.
fn caller(
    state: &AppState,
    headers: &HeaderMap,
    need_admin: bool,
) -> Result<(KeyRef, Arc<KeyStore>), Refusal> {
    let Some(store) = state.keys.clone() else {
        return Err((
            StatusCode::NOT_FOUND,
            code::AUTH_OFF,
            "this server runs without keys (auth = \"none\")",
        ));
    };
    let Ok(Some(key)) = crate::authorize(state, headers) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            code::UNAUTHORIZED,
            "unknown or revoked key",
        ));
    };
    if need_admin && !key.admin {
        tracing::info!(key = %key.id, "admin: refused, not an admin key");
        return Err((
            StatusCode::FORBIDDEN,
            code::FORBIDDEN,
            "this key is not an admin key",
        ));
    }
    Ok((key, store))
}

fn refused((status, c, m): Refusal) -> Response {
    error(status, c, m)
}

fn view(k: &KeyInfo, current: &str) -> KeyView {
    KeyView {
        id: k.key.id.clone(),
        name: k.key.name.clone(),
        admin: k.key.admin,
        created: k.key.created,
        revoked: k.key.revoked,
        last_seen: k.usage.last_seen,
        client: k.usage.client.clone(),
        dictations: k.usage.dictations,
        audio_ms: k.usage.audio_ms,
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

async fn whoami(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match caller(&state, &headers, false) {
        Ok((k, _)) => json(
            StatusCode::OK,
            &WhoAmI {
                id: k.id,
                name: k.name,
                admin: k.admin,
            },
        ),
        Err(r) => refused(r),
    }
}

async fn list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let (me, store) = match caller(&state, &headers, true) {
        Ok(x) => x,
        Err(r) => return refused(r),
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

async fn create(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let (me, store) = match caller(&state, &headers, true) {
        Ok(x) => x,
        Err(r) => return refused(r),
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
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (me, store) = match caller(&state, &headers, true) {
        Ok(x) => x,
        Err(r) => return refused(r),
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
