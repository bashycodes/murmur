# Hybrid local/server architecture

Status: design (branch `hybrid-design`), decision D17 in `docs/DECISIONS.md`.
Covers the `core-p1` API (segments, `plain_text`, `empty_reason`, typed load errors,
`vad`/`hallucination_guard`/`spoken_cleanup`/`llm_prefill` session flags).

**Goal.** A shell (Windows, Android) dictates either on-device ("client", today's path)
or by streaming to a self-hosted `flow-server` on the user's own machine (i7-6700, 8
threads, 19 GB RAM, no GPU; LAN or VPN). Voice never reaches a third party.
**Hard requirement:** going server-only or client-only later must be a build-flag
change with no integration work. Proof: the toggle-matrix and equivalence suites (§4)
and the per-flavour build gates (§1.4).

**Invariants**

| # | Invariant | Enforced by |
|---|---|---|
| I1 | Shells talk to exactly one API: the §9 session API, served by `flow_router::Router` in every build | §1, §6 ownership |
| I2 | Remote results are identical to local results for the same ASR model and inputs (the server runs the full flow-core pipeline; the shell's `SessionConfig`/`FieldContext` pass through verbatim) | `equivalence.rs` |
| I3 | Client-only builds contain no network code and (Android) no network permission | `cargo tree` gate, `verifyLocal*NoNetworkPermissions` |
| I4 | Server-only builds contain no ASR/LLM native libs and need no models | `verifyRemote*NoAsrLibs`, `tools/check-build-matrix.sh` |
| I5 | Audio pushed during a remote session is never lost on fallback | router PCM buffer, `matrix_*_dies_mid_session_*` |
| I6 | The server persists neither audio nor text; it keeps no per-user state | §3.5, `flow-server/tests/no_persist.rs` |

---

## 1. Engine abstraction

### 1.1 Where it lives: Rust, below both shells

| Option | Verdict |
|---|---|
| Router + WS client in each shell (Rust in Windows, Kotlin in Android) | Rejected. Two routers (fallback, buffering, timeouts, circuit breaker) and two protocol clients would drift; Android has no built-in WebSocket client (no AndroidX/OkHttp in this app) |
| **Router + WS client in Rust crates, reached by Windows directly and by Android via flow-ffi/JNI** | **Chosen.** One implementation, tested once by the contract suite (§4); the protocol is spoken only by Rust on both ends, so there is no Kotlin wire mirror to keep in sync. Kotlin only parses router JSON (config, health, route info), pinned by shared fixtures |

New crates (root workspace):

| Crate | Depends on | Network code | Purpose |
|---|---|---|---|
| `crates/flow-protocol` | flow-core (`default-features = false`), serde | no | Wire message types, version constants, golden fixtures |
| `crates/flow-router` | flow-core (`default-features = false`) | **no** | `Router` (session API mirror), `Backend` trait, `RouterConfig`, policy, PCM buffer, circuit breaker, `impl Backend for FlowCore` |
| `crates/flow-remote` | flow-protocol, flow-router, tokio, tokio-tungstenite, rustls (feature `tls`) | yes | `RemoteBackend` (WS client) + `flow_remote::factory()` |
| `crates/flow-server` | flow-core, flow-protocol, axum, tokio | yes | The server binary (§3) |
| `crates/flow-contract-tests` | all of the above (dev, `publish = false`) | test only | §4 suites |

`flow-router` takes the remote backend as an injected factory, so a client-only build
links the router but not `flow-remote`: the network code is absent, not just unused.

### 1.2 Router API (frozen here; Windows and Android code against it with stubs)

```rust
// crates/flow-router/src/lib.rs
pub enum EngineKind { Local, Remote }            // serde: "local" | "remote"
pub enum Prefer { Server, Client }               // serde: "server" | "client"

#[serde(default)] pub struct RouterConfig {
    pub allow_server: bool, pub allow_client: bool, pub prefer: Prefer,   // Server
    pub server: ServerConfig, pub keep_local_warm: bool,                  // true
}
#[serde(default)] pub struct ServerConfig {
    pub host: String, pub port: u16 /*7313*/, pub tls: bool, pub tls_pin_sha256: Option<String>,
    pub token: String, pub connect_timeout_ms: u64 /*2500 (D24; was 800)*/, pub commit_timeout_base_ms: u64 /*3000*/,
    pub health_ttl_s: u64 /*30*/,
}
pub struct Capabilities { pub local: bool, pub remote: bool, pub asr_engines: Vec<String>,
                          pub llm: bool, pub proto: u32, pub core_version: String }

pub trait Backend: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn ready(&self) -> bool;                                    // local: models loaded
    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError>;
    fn push_audio(&self, pcm: &[i16]) -> bool;                  // non-blocking
    fn set_context(&self, ctx: &FieldContext);
    fn partial(&self) -> Option<String>;
    fn commit(&self, deadline: std::time::Instant) -> Result<CommitResult, BackendError>;
    fn cancel(&self);
    fn probe(&self) -> HealthReport;                            // local: ready; remote: hello + 3 pings
}
pub type RemoteFactory = Box<dyn Fn(&ServerConfig) -> Arc<dyn Backend> + Send + Sync>;

impl Router {
    pub fn new(cfg: RouterConfig, local: Option<Arc<dyn Backend>>, remote: Option<RemoteFactory>) -> Self;
    pub fn update_config(&self, cfg: RouterConfig);             // takes effect at next start_session
    pub fn set_local(&self, local: Option<Arc<dyn Backend>>);   // shell (re)loaded models
    pub fn capabilities(&self) -> Capabilities;
    // §9 mirror, same semantics as FlowCore:
    pub fn start_session(&self, cfg: SessionConfig) -> Result<(), RouterError>;
    pub fn push_audio(&self, pcm: &[i16]) -> bool;
    pub fn set_context(&self, ctx: FieldContext);
    pub fn partial(&self) -> Option<String>;
    pub fn poll(&self) -> SessionPoll;                          // partial + hands-free endpoint (D19)
    pub fn endpoint_reached(&self) -> bool;                     // computed in the router, every route
    pub fn commit(&self) -> CommitResult;                       // never fails; `route` filled
    pub fn cancel(&self);
    pub fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<Candidate>;
    pub fn probe_server(&self) -> HealthReport;                 // blocking; dev "Test connection"
    pub fn route_status(&self) -> RouteStatus;                  // last RouteInfo + circuit state
}
```

* `learn_edit` always runs locally: `flow_core::learn::learn_edit(pasted, edited, &last_session_dictionary)`. It is pure and needs no model, so it behaves the same in every build and needs no wire message.
* `RouterError` kinds (stable snake_case, surfaced as last error): `no_engine_enabled`, `local_not_ready`, `server_not_configured`, `server_unavailable`, `bad_config`.

**Additive flow-core change** (the only one): `CommitResult.route: Option<RouteInfo>`
(`#[serde(default, skip_serializing_if = "Option::is_none")]`) with

```rust
pub struct RouteInfo { pub engine: String /*"local"|"remote"*/, pub fallback: Option<String>,
    pub server: Option<String> /*"host:port"*/, pub rtt_ms: Option<u32>, pub queue_ms: Option<u32>,
    pub server_core: Option<String> }
```

`fallback` values: `connect_failed`, `auth_failed`, `proto_mismatch`, `busy`, `timeout`,
`disconnected`, `circuit_open`, `server_not_configured`, `local_not_ready` (client-first
→ server). FlowCore itself never sets `route`; the router does. Also add
`pub const POSTPROCESS_REV: &str` to flow-core (bump whenever deterministic output
changes) so client and server can detect pipeline skew.

### 1.3 Routing policy

Effective flags = build capability AND user toggle (`allow_server_eff = caps.remote && cfg.allow_server`).

| allow_server_eff | allow_client_eff | Primary | Fallback | Router at `start_session` |
|---|---|---|---|---|
| on | on | `prefer` (default server) | the other | see failure table |
| on | off | remote | none | failure → `commit` returns ERROR, `route.fallback` set, error `server_unavailable: <why>` |
| off | on | local | none | **never constructs a remote backend, never opens a socket** |
| off | off | none | none | `start_session` → `Err(no_engine_enabled)`; shell shows "Dictation disabled: turn on Allow server or Allow client in Developer settings" |

Server with an empty `host` counts as `server_not_configured`: treated as remote failure
(falls back if client allowed; Developer page shows a warning row).

**Remote-primary session flow (both allowed)**

1. `start_session`: if the circuit is open (failure < backoff ago: 5 s, doubling to 60 s) → local immediately, `fallback=circuit_open`, re-probe in the background. Else reuse the warm WS (kept `idle_keepalive_s`=60 after the last session) or connect within `connect_timeout_ms`. Send `hello` (new connection) + `start`; do **not** wait for `started` before accepting audio (pipelined).
2. Every `push_audio`: append to the router's session PCM buffer (`Vec<i16>`, capped at `max_samples`; only allocated when a fallback exists) and `try_send` to the WS writer task (bounded 64 frames ≈ 6.4 s). Never blocks.
3. `set_context`: store the latest context in the router; forward.
4. `partial`: last `partial` message from the server (or local backend after a switch).
4b. `poll` / `endpoint_reached` (D19): the router runs `flow_core::endpoint::Endpointer` on the audio it is pushed, so the hands-free endpoint is the same on every route, needs no protocol message, and survives a fallback replay. `SessionConfig.endpoint_silence_ms` still reaches the server unchanged (its own detector is simply not read).
5. `commit`: send `commit`, wait until `deadline = now + commit_timeout_base_ms + 0.3 × audio_ms + llm_timeout_ms`.

**Failure handling (fallback = local allowed and ready)**

| When | Trigger | Action | `route.fallback` |
|---|---|---|---|
| start | connect error/timeout, TLS/pin error | local session from scratch | `connect_failed` |
| start | WS close 4001 / `error unauthorized` | local; circuit open; **not retried** until config changes | `auth_failed` |
| start | `hello` proto range disjoint | local; circuit open until config changes | `proto_mismatch` |
| start | `started{queued:true}` or `error busy` | send `cancel`; local | `busy` |
| speaking | WS error/close, writer queue full, no `started` within `connect_timeout_ms` | `local.start_session(cfg)`, replay buffer, `set_context(last)`, keep pushing live audio to local | `disconnected` |
| commit | deadline passed / WS dies | `local.start_session`, replay full buffer, `set_context`, `local.commit()` | `timeout` / `disconnected` |
| commit | well-formed `result` (any status, incl. ERROR) | **final**, no fallback (determinism: never two answers for one utterance) | none |

Local not ready at fallback time → ERROR `server_unavailable: … ; local model not ready`.
`keep_local_warm=true` (default) means the shell loads local models at app start even
when the server is preferred, so a fallback costs only local ASR time.
**D24:** with `keep_local_warm=false` the shell gives the router an on-demand loader
(`Router::with_local_loader` / `set_local_loader`), so "local not ready" no longer fails a
dictation: the session goes live on the client route at once, audio is buffered while the
loader runs on the failover thread ("Loading phone model…"), then it is replayed (see the
2026-10-23 D24 changelog entry).

**Client-first** (`prefer=client`, both allowed): local is primary; remote is used only when
`local.ready()` is false at `start_session` (`fallback=local_not_ready`). Mid-session
local failures do not exist in practice (in-process), so there is no mid-session switch.

**Health.** `probe()` = connect + `hello` + 3 × `ping`/`pong`; reports min/median/max RTT.
Cached for `health_ttl_s`; refreshed on `update_config`, at app start, and after any
failure. Sessions do not wait for a probe; the circuit breaker uses the last outcome.

**Cloud text cleanup (D28).** A second, separate decision runs at commit time inside the
engine that produced the transcript. On the client and cloud-STT routes, the router installs
`RoutedCleaner(OpenRouter gate, on-device LLM)` per `cleanup_order`. The server route keeps
its own pipeline and never calls cloud cleanup (no egress, D17 I6). The gate re-checks C1 for
cleanup on every call (toggle, order, cleanup consent, key, circuit, budget). It refuses fast,
so the on-device LLM or the deterministic text is used. The cloud call has a hard
`cleanup.timeout_ms` (2500 ms) and its stream is cancelled at the deadline. Details: "Changelog
for shells", D28 entry.

### 1.4 Build variants

**Windows** (`apps/flow-windows/Cargo.toml` features):

| Feature | Pulls in | Notes |
|---|---|---|
| `local` (default) | `whisper` (existing) + optional `parakeet` | local backend = existing `FlowCoreAdapter` |
| `remote` (default) | `flow-remote` | |
| `app` | Tauri (unchanged) | |

Client-only: `--no-default-features --features local,app`. Server-only:
`--no-default-features --features remote,app` (no whisper, no `flow_llm.dll`, no model
download hints). The shell always builds a `Router`; `#[cfg(feature)]` only decides
whether `local`/`remote` are `Some`.

**Android** product flavours, dimension `net`; same `applicationId dev.flowclone` for all
three so switching is an in-place update that keeps prefs, dictionary and history:

| Flavour | `libflow_ffi.so` features | Extra libs | Manifest | Gates |
|---|---|---|---|---|
| `local` | `jni,whisper,llm[,parakeet]` | libc++_shared, libflow_llm, [sherpa, onnxruntime] | today's (network perms removed) | `verifyLocal<BT>NoNetworkPermissions` (existing task, now flavour-scoped) |
| `hybrid` | `jni,whisper,llm,remote[,parakeet]` | as local | `src/hybrid/AndroidManifest.xml` adds **only** `INTERNET` | `verifyHybrid<BT>OnlyInternetPermission` |
| `remote` | `jni,remote` | none | adds only `INTERNET` | `verifyRemote<BT>OnlyInternetPermission`, `verifyRemote<BT>NoAsrLibs` (APK has no libwhisper/libsherpa*/libonnxruntime/libflow_llm/libc++_shared; `libflow_ffi.so` exports no `whisper_`/`llama_` symbols) |

* jniLibs move from `src/main/jniLibs` to `src/<flavour>/jniLibs`; `tools/build-native.sh --flavor local|hybrid|remote` fills them.
* `BuildConfig.CAP_LOCAL` / `CAP_REMOTE` per flavour; at startup they must equal `nativeCapabilities()` (mismatch = main-screen error "native library flavour mismatch").
* `remote` flavour hides model import / LLM GGUF import; `ModelConfig` is never built.
* Native sockets are not subject to Android's cleartext policy (Network Security Config covers Java stacks only), so `ws://` to a VPN address works without `usesCleartextTraffic`. The router's own address check (§2.6) is the gate.
* A mesh VPN on Android is an OS VPN: no extra permission or code.

**Root workspace gate** `tools/check-build-matrix.sh` (owner: server implementer):
`cargo tree -p flow-ffi -e normal --features jni,whisper` must not contain
`tokio-tungstenite|tokio|rustls|axum`; `--features jni,remote` must not contain
`whisper-rs-sys|sherpa-onnx|llama-cpp`. Same two checks for `apps/flow-windows` with
`local,app` and `remote,app`. Run by `cargo test -p flow-contract-tests --test build_matrix`
(shells out to the script).

---

## 2. Wire protocol v1 (ours; Wispr's proto is not used, D8)

Spec file: `docs/protocol.md` (normative; this section is its outline). Types:
`crates/flow-protocol/src/lib.rs`. Golden fixtures: `crates/flow-protocol/fixtures/*.json`.

### 2.1 Transport

* `GET /v1/stream` → WebSocket, subprotocol `flow.v1`. `Authorization: Bearer <token>` on the upgrade request (fallback `?token=` is **not** offered: tokens must not land in access logs).
* `GET /v1/health` (same auth) → `HelloOk` JSON, for `curl`/monitoring.
* Path major `/v1/` changes only on breaking change; minor `proto` integer is negotiated in `hello`.
* TCP_NODELAY on both ends. WS-level ping every 15 s; 45 s silence = dead.
* One WS = at most one active session (mirrors one `FlowCore` handle). The client keeps the WS open between sessions for `idle_keepalive_s` (60 s).

### 2.2 Frames

* **Binary frame = audio** for the current session: raw PCM16 little-endian, mono, 16 kHz, even length, ≤ 64 KiB (recommended 100 ms = 3200 bytes). No header; ordering is TCP's. Audio with no active session is dropped and counted (mirrors `push_audio` → false), not an error.
* **Text frame = JSON control**, tagged by `"t"`. Unknown fields ignored (no `deny_unknown_fields` anywhere); unknown `t` → non-fatal `error bad_request`.

### 2.3 Messages (1:1 with the session API)

| Client → server | Server → client | Session API |
|---|---|---|
| `{"t":"hello","proto_min":1,"proto_max":1,"client":"flow-android 0.2.0","core":"0.1.0","postprocess_rev":"…"}` | `{"t":"hello_ok","proto":1,"server":"flow-server 0.2.0+1a2b3c4","core":"0.1.0","postprocess_rev":"…","asr_backend":"parakeet","asr_model":"sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8","llm":false,"workers":1,"busy":0,"max_samples":19200000}` | (handshake) |
| `{"t":"start","sid":7,"config":<SessionConfig>}` | `{"t":"started","sid":7,"queued":false}` | `start_session` |
| *binary PCM* | — | `push_audio` |
| `{"t":"context","sid":7,"ctx":<FieldContext>}` | — | `set_context` |
| — | `{"t":"partial","sid":7,"text":"ask not"}` (pushed when changed; only if `partial_interval_ms` set) | `partial` |
| `{"t":"commit","sid":7}` | `{"t":"result","sid":7,"result":<CommitResult>,"server_timings":{"queue_ms":0,"recv_audio_ms":11000}}` | `commit` |
| `{"t":"cancel","sid":7}` | `{"t":"cancelled","sid":7}` | `cancel` |
| `{"t":"ping","n":3}` | `{"t":"pong","n":3,"busy":0}` | (health) |
| — | `{"t":"error","sid":7?,"code":"…","message":"…","fatal":false}` | |

* `<SessionConfig>`, `<FieldContext>`, `<CommitResult>` are **flow-core's serde types verbatim** (flow-protocol re-exports them). So `segments`, `plain_text`, `empty_reason`, the core-p1 flags (`vad`, `hallucination_guard`, `spoken_cleanup`, `llm_prefill`), `platform`, dictionary, styles, `llm_timeout_ms` and any future additive field flow through with no protocol change. The server never sets `route`.
* `sid`: client-chosen u64, increasing per connection. Messages with a stale `sid` → non-fatal `error stale_sid`. A `start` while a session is active cancels it (as `FlowCore::start_session`).
* **Dictionary / learn_edit sync: none needed.** The server is stateless: the dictionary travels in every `start.config`; `learn_edit` runs client-side (§1.2). The server stores no user data.
* `ModelConfig` is server-side config only; a client cannot choose server paths.
* Error codes: `unauthorized` (also WS close 4001), `proto_mismatch` (close 4002), `busy`, `bad_request`, `stale_sid`, `no_session`, `too_large` (frame > 64 KiB or session > `max_samples`; sets `truncated` like the core instead when only over `max_samples`), `server_error`. `fatal:true` = server closes after sending.

### 2.4 Versioning rules

* `proto` = 1. Server supports `[1, PROTO_MAX]`; picks `min(client.proto_max, server max)` if ≥ both mins, else `proto_mismatch`.
* Additive changes (new optional fields, new message types the peer may ignore, new error codes) do **not** bump `proto`. Bump `proto` when a peer must understand something new; bump the path to `/v2/` only for a breaking change.
* `core`/`postprocess_rev` differences are not fatal: the router records `route.server_core`, and the Developer page warns "server pipeline ≠ client pipeline; results may differ".

### 2.5 Auth

* Named keys (D38, `src/keys.rs`): each person or device has its own bearer secret, managed with `flow-server keys add <name> [--admin] | list | revoke <name|id>`. `keys.json` in the state dir stores only SHA-256 hashes + metadata; `key-usage.json` per-key counters. The old `token_file` is imported once as the admin key `owner`; the server re-reads `keys.json` on change, so a revoke applies to the next connection. Details: `docs/server-setup.md` §4. Admin API (D39, `src/admin.rs`): admin keys list / create / revoke keys over `/v1/admin/keys`; Windows Settings → Users & keys uses it through `flow_remote::admin`.
* Originally one shared bearer token: `flow-server gen-token` writes 32 random bytes base64url (43 chars) to `token_file` (mode 0600). Constant-time compare. Clients store it (Windows: DPAPI `CryptProtectData` blob in prefs; Android: Android Keystore AES-GCM key `flow_dev_token`, ciphertext in the `dev` prefs file). Minimum accepted length 16.
* The server refuses to start without a token or a keys file unless `auth = "none"` **and** every bind address is loopback.

### 2.6 Transport security

| Server address (client-side check, `flow_router::net::classify`) | `ws://` | `wss://` |
|---|---|---|
| loopback, RFC 1918, 100.64.0.0/10 (VPN/CGNAT), fd7a:115c:a1e0::/48 (VPN overlay), fc00::/7, link-local, `*.ts.net`, `*.local`, single-label names | allowed; Developer page shows "unencrypted: OK only on your LAN or over an encrypted VPN" | allowed |
| anything else (public IP or public DNS name) | **refused** (`connect_failed: plaintext to public address`) | required |

Recommended: a WireGuard-based VPN (it already encrypts; bind the server to its 100.x address).
TLS (server `[tls] cert/key`, rustls) for LAN without a VPN; self-signed certs pinned
with `tls_pin_sha256` (SHA-256 of the leaf cert DER, 64 hex). Hostnames resolve to the
user-entered server only; no discovery, no relay, no fallback hosts.

### 2.7 Where post-processing runs

Remote mode: **everything on the server** (VAD → ASR → hallucination guard → spoken
cleanup → optional LLM → guard → Wispr post-processing → segments), via an unmodified
`FlowCore` session fed the client's `SessionConfig`/`FieldContext`. The client does no
text shaping, so remote and local differ only if the ASR/LLM models differ.

---

## 3. `crates/flow-server`

### 3.1 Structure

| File | Content |
|---|---|
| `src/main.rs` | CLI: `flow-server [--config PATH]`, `gen-token`, `check-config`, `keys` |
| `src/keys.rs` | Named keys (D38): hashed key store, usage counters, hot reload |
| `src/config.rs` | TOML config (below), validation |
| `src/pool.rs` | `workers` × `FlowCore`, loaded at startup; lease/return; queue |
| `src/ws.rs` | per-connection task: auth, hello, message loop, partial poller |
| `src/health.rs` | `/v1/health` |
| `src/lib.rs` | `pub async fn serve(cfg, listener) -> ServerHandle` (used by tests in-process) |
| `dist/flow-server.service`, `dist/config.example.toml` | systemd unit, example config |
| `tests/ws_protocol.rs`, `tests/auth.rs`, `tests/queue.rs`, `tests/no_persist.rs` | crate tests (mock ASR) |

### 3.2 Config (`/etc/flow-server/config.toml`)

```toml
bind = ["127.0.0.1:7313", "100.64.0.10:7313"]  # default ["127.0.0.1:7313"]
token_file = "/etc/flow-server/token"           # imported as key "owner" (D38); needed unless keys.json exists or auth="none" on loopback
# state_dir = "/var/lib/flow-server"            # keys.json + key-usage.json (default $STATE_DIRECTORY / XDG state dir)
workers = 1            # FlowCore instances; each holds its models (Parakeet ≈ 0.9 GB RSS)
max_queue = 2          # sessions waiting for a worker
queue_timeout_ms = 30000
idle_timeout_s = 120   # close idle WS
max_session_s = 1200   # 20 min, same as the Windows hard stop
[model]                # = flow_core::ModelConfig verbatim
asr_engine = "parakeet"
asr_path = "/var/lib/flow-server/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"
threads = 4            # 4 on the i7-6700: 8 threads was 14x slower (HT)
# llm_path = "/var/lib/flow-server/models/qwen2.5-1.5b-instruct-q4_k_m.gguf"
# vad_model_path = …
[tls]                  # optional
# cert = "…"; key = "…"
[log]
level = "info"
log_text = false       # true prints transcripts at debug level; startup warns loudly
```

Build features mirror flow-core: `whisper` (default), `parakeet`, `llm`. Default
`workers = 1` because the box has 4 physical cores and one 4-thread decode saturates
them; RAM would allow 2-3 (set `workers = 2` and `threads = 2` to trade latency for
concurrency).

### 3.3 Concurrency

* A session leases a worker at `start` and returns it at `result`/`cancelled`/disconnect (`FlowCore::cancel`). Lease at start (not at commit) keeps partials and LLM prefill-ahead.
* No free worker: `started{queued:true}`; audio is buffered server-side (bounded `max_samples`) and replayed when a worker frees; `queue_timeout_ms` → `error busy fatal:false` and the session ends. Queue full → `error busy` immediately. (Routers with a client fallback cancel on `queued:true`, §1.3.)
* Each `FlowCore` call that blocks (`commit`) runs in `spawn_blocking`; `push_audio` is called directly (non-blocking ring).
* Partial poller: every 100 ms while a session has `partial_interval_ms`, `FlowCore::partial()`; send if changed.

### 3.4 Startup and systemd

* Models load before the listener binds (`load_models` fails → exit 1 with `CoreError::kind()`; systemd restarts). Log line: `ready workers=1 asr=parakeet load_ms=…`.
* `dist/flow-server.service`: `DynamicUser=yes`, `StateDirectory=flow-server`, `Environment=TMPDIR=/var/lib/flow-server/tmp` (Parakeet `bpe.vocab`), `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`, **`IPAddressDeny=any` + `IPAddressAllow=localhost 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 169.254.0.0/16 fc00::/7 fe80::/10`** (the kernel enforces "no third party"; the server applies the same classes itself at accept, so a manual start or a container is not exposed), `MemoryMax=8G`, `Restart=on-failure`, `Nice=5`.

### 3.5 Privacy and logging

* Never written to disk: audio, transcripts, contexts, dictionaries. No audio dump option exists in v1. History lives only on the client.
* Default log per session (tracing → journald): `sid`, peer address, `audio_ms`, `asr_ms`, `llm_ms`, `total_ms`, `queue_ms`, `status`, `empty_reason`, `fallback_reason`. No text unless `log_text=true`.
* Proof `tests/no_persist.rs`: runs a session with `HOME`/`TMPDIR`/cwd pointed at an empty temp dir and asserts it is still empty afterwards, and that captured log output does not contain the mock transcript.
* The server makes no outbound connections (only listens); the systemd IP allowlist backs this up.

---

## 4. Equivalence and toggle-matrix proof

All in `crates/flow-contract-tests` (`cargo test -p flow-contract-tests`). Harness
`src/lib.rs`: `spawn_server(cfg) -> TestServer` (in-process `flow_server::serve` on
`127.0.0.1:0`, mock or real models), `TestServer::{kill(), stall_commits(), set_busy(), connections()}`,
`local_backend(model)`, `remote_router(addr)`, `normalize(&CommitResult) -> serde_json::Value`
(drops `timings` and `route`; everything else compared byte-for-byte as JSON).

Fixtures: `crates/flow-contract-tests/fixtures/sessions/*.json`:
`{name, asr: {"mock":"text"}|"whisper-tiny"|"parakeet", audio: "jfk.wav"|"silence:3000"|"noise:3000"|"noise+jfk", session_config, contexts:[{at_ms, ctx}], learn?: {pasted, edited}}`.
≥ 40 cases generated once by `tools/gen-session-fixtures.py` from `crates/flow-core/tests/vectors/` (cases expressible as ASR text + config + context: dictionary, cursor context, snippets, press-enter, `{{BREAK}}`/`{{TAB}}`, android platform gates, styles) plus core-p1 cases (spoken cleanup, `only_filler`, `no_speech`, hallucination).

**`tests/equivalence.rs`** — each runs the case through `FlowCore` directly (local) and through `Router{allow_server only}` → `flow-remote` → in-process server (remote), and asserts `normalize(local) == normalize(remote)`:

| Test | What |
|---|---|
| `equivalence_session_fixtures_mock_asr` | all session fixtures, MockAsr per case |
| `equivalence_empty_reasons` | `no_audio`, `no_speech`, `blank_asr`, `hallucination`, `only_filler` |
| `equivalence_segments_and_press_enter` | segments/plain_text/press_enter identical |
| `equivalence_late_context` | context sent after audio, before commit |
| `equivalence_cancel_then_commit_is_error` | cancel → commit = ERROR both ways |
| `equivalence_max_samples_truncated` | `truncated` + `dropped_samples` semantics |
| `equivalence_partials_mock` | partial stream equals local partial sequence (mock) |
| `equivalence_learn_edit` | router `learn_edit` == `FlowCore::learn_edit` with the same dictionary |
| `equivalence_whisper_tiny_jfk_if_present` | real whisper tiny.en on jfk.wav (skip without `models/`) |
| `equivalence_parakeet_if_present` | real Parakeet (skip without the model) |
| `equivalence_llm_if_present` | real LLM with generous budget (skip without plugin/GGUF) |

**`tests/toggle_matrix.rs`** — 4 toggle combos × server state {`up`, `down`, `busy`, `stalls_commit`, `dies_mid_session`, `bad_token`}; MockAsr records total samples received so loss is measurable. Naming `matrix_<server>_<client>_<state>_<expectation>`:

| Test | Expect |
|---|---|
| `matrix_on_on_up_uses_remote` | route remote, no fallback |
| `matrix_on_on_down_falls_back_local` | route local, `connect_failed`, start→result ≤ connect_timeout + local ASR |
| `matrix_on_on_busy_goes_local` | `busy`; server received `cancel` |
| `matrix_on_on_stalls_commit_times_out_to_local` | `timeout`; local result text == full-audio result |
| `matrix_on_on_dies_mid_session_replays_buffer` | `disconnected`; local saw 100 % of pushed samples, `dropped_samples=0`, text == no-failure run |
| `matrix_on_on_bad_token_auth_failed_not_retried` | `auth_failed`; 2nd session opens no connection |
| `matrix_on_on_circuit_open_skips_remote` | 2nd session within backoff: `circuit_open`, 0 new connections |
| `matrix_on_on_prefer_client_uses_local` / `…_local_not_ready_uses_remote` | client-first semantics |
| `matrix_on_off_up_uses_remote` | |
| `matrix_on_off_down_is_error_server_unavailable` | ERROR, `route.fallback=connect_failed` |
| `matrix_on_off_dies_mid_session_is_error` | ERROR, no hang past deadline |
| `matrix_off_on_up_never_connects` / `…_down_never_connects` | `TestServer::connections()==0`; also constructed with `remote: None` |
| `matrix_off_off_start_fails_no_engine_enabled` (× up/down) | `Err(no_engine_enabled)`, commit = ERROR |
| `matrix_update_config_applies_next_session` | toggle flip mid-session affects only the next one |
| `matrix_plaintext_public_host_refused` | `ws://8.8.8.8` refused without a socket |

**Other suites**

| File | Owner | What |
|---|---|---|
| `crates/flow-protocol/tests/fixtures.rs` | server | every `fixtures/*.json` parses and re-serialises to the same JSON; unknown-field tolerance; proto negotiation table |
| `crates/flow-router/tests/policy.rs` | server | policy table with fake backends (no network) |
| `crates/flow-router/fixtures/*.json` + `tests/fixtures.rs` | server | `router_config_{default,hybrid,remote_only,local_only}.json`, `capabilities_{local,hybrid,remote}.json`, `health_ok.json`, `health_auth_failed.json`, `route_status.json`, `commit_result_remote.json`, `commit_result_fallback.json` |
| `apps/flow-android/app/src/test/.../core/RouterJsonTest.kt` | android | parses the same `crates/flow-router/fixtures` (Gradle `sourceSets.test.resources.srcDir("../../../crates/flow-router/fixtures")`, no copies) and serialises `DevSettings` → JSON equal to `router_config_*.json` |
| `apps/flow-windows/tests/router_integration.rs` | windows | controller + `Router` + in-process server (dev-dep on flow-server, mock ASR): PTT → remote → paste; kill server → local paste; both off → error pill |
| `crates/flow-contract-tests/tests/build_matrix.rs` | server | runs `tools/check-build-matrix.sh` |
| `crates/flow-ffi/tests/jni_smoke.rs` (extend) | android | `nativeOpenRouter` round trip with mock local + in-process server |

---

## 5. Developer settings (both apps, same keys)

Settings serialise to `RouterConfig` JSON (§1.2) — the fixtures are the spec. UI-only keys
are marked (ui).

| Key | Type / control | Default | Validation |
|---|---|---|---|
| `allow_server` | toggle | hybrid/remote builds: on; local build: off and disabled ("not in this build") | — |
| `allow_client` | toggle | local/hybrid: on; remote build: off and disabled | — |
| `prefer` | segmented "Server first / Client first" (visible when both on) | server | — |
| `server.host` | text | "" | trim; no scheme/path/port/whitespace; one of: IPv4 dotted quad (4 parts 0-255, no leading zeros), IPv6 literal (brackets optional, stored without), hostname (labels 1-63 `[A-Za-z0-9-]`, no leading/trailing `-`, total ≤ 253). Inline error text; Save disabled while invalid |
| `server.port` | number | 7313 | integer 1-65535 |
| `server.tls` | toggle "Use TLS (wss)" | off | forced on (and locked) when host classifies public (§2.6) |
| `server.tls_pin_sha256` | text (shown when TLS on) | "" | empty or 64 hex (colons stripped) |
| `server.token` | password field + show/paste/clear | "" | empty or ≥ 16 chars; never logged, never in exported settings |
| `server.connect_timeout_ms` | number | 2500 (D24; a stored 800 migrates once) | 100-10000; the router raises it to `rtt × 6` (1.5-10 s) after a probe measured a slow path |
| `server.commit_timeout_base_ms` | number | 3000 | 500-60000 |
| `server.health_ttl_s` | number | 30 | 5-600 |
| `keep_local_warm` | toggle | on | — |
| `show_engine_badge` (ui) | toggle | off | overlay pill / bubble shows `S`/`L` (+ red dot on fallback) |

Read-only rows:
* **Build:** `capabilities` (local/remote, ASR engines, LLM, proto, core version).
* **Active engine (last dictation):** `route.engine`, `fallback`, server, `rtt_ms`, `queue_ms`, `asr_ms`/`total_ms`, status, `empty_reason`, timestamp. In-memory only.
* **Server info** from the last probe: server/core version, `postprocess_rev` match (✓/warning), ASR backend + model, LLM on/off, workers/busy.
* **Circuit:** closed / open until hh:mm:ss (reason).

Buttons:
* **Test connection** → `probe_server()` with the form's unsaved values, off the UI thread. Shows "OK · median 4 ms (3-6) · flow-server 0.2.0+1a2b3c4 · parakeet" or the error code with a hint (`auth_failed` → "token rejected", `connect_failed` → "is flow-server running / LAN or VPN up?", `proto_mismatch` → versions).
* **Test session** → 1 s silence through the full router path with the current toggles; expects EMPTY `no_speech`; shows the route used.
* **Reset circuit**, **Reset to defaults**.

Errors in normal UI (not only the dev page): both toggles off → dictation pill/bubble error
"Dictation disabled — enable server or client in Developer settings"; server-only failure →
"Server unreachable (<reason>)".

**Access.** Visible by default in builds with `remote` (hybrid/remote: the server must be
configurable). Client-only builds: hidden until the version row in Settings is tapped
7 times ("Developer settings unlocked"), persisted `dev_unlocked`; "Lock" button hides it.

**Persistence.**
* Windows: `prefs.json` key `"developer": {…RouterConfig minus token…, "token_dpapi": "<base64>", "dev_unlocked": bool, "show_engine_badge": bool}`, through the existing tolerant load / atomic save in `prefs.rs`. Tab "Developer" in the settings window (`ui/src/settings.ts`), Tauri commands `dev_get`, `dev_set`, `dev_test_connection`, `dev_test_session`, `dev_route_status`.
* Android: SharedPreferences file `dev` (backup already disabled), `data/DevSettings.kt` (data class + `toRouterConfigJson()` + validation, JVM-tested), `ui/DeveloperActivity.kt`, token via Keystore (`data/TokenVault.kt`).

**Settings → router.** On save: validate → build `RouterConfig` → `router.update_config(cfg)`
(Windows direct; Android `nativeUpdateRouter`). Applies from the next `start_session`;
never reloads local models. Local model/LLM pref changes keep using the existing reload
paths, then call `router.set_local(..)` (Windows) / re-open the router with the new local
handle (Android). A probe runs after each save; its result fills "Server info".

---

## 6. Work split

### 6.1 Ownership (exclusive write access)

| Implementer | Owns | Must not touch |
|---|---|---|
| **S: server + client stack** | `crates/flow-protocol/**`, `crates/flow-router/**`, `crates/flow-remote/**`, `crates/flow-server/**`, `crates/flow-contract-tests/**`, `docs/protocol.md`, `tools/check-build-matrix.sh`, `tools/gen-session-fixtures.py`, root `Cargo.toml` members; the additive flow-core change (`CommitResult.route`, `RouteInfo`, `POSTPROCESS_REV`) | `apps/**`, `crates/flow-ffi/**` |
| **W: Windows** | `apps/flow-windows/**` (Cargo features, `src/router_adapter.rs` new: `CoreApi` on `flow_router::Router`, local backend wrapper over `FlowCoreAdapter`'s slot, `prefs.rs` `developer`, `app.rs` commands, `ui/src/settings.ts` Developer tab, `tests/router_integration.rs`, README section) | `crates/**` |
| **A: Android + FFI** | `apps/flow-android/**` (flavours, gates, `DevSettings`, `TokenVault`, `DeveloperActivity`, `NativeFlowCore` additions, `CoreJson` new fields incl. core-p1 `segments`/`plain_text`/`empty_reason`/`route`, `RouterJsonTest`, `build-native.sh --flavor`, JNI_CONTRACT.md, INSTALL.md), `crates/flow-ffi/**` (router handle, `remote` feature, C + JNI additions, header) | other `crates/**` |

Changes outside one's area go through `docs/core-change-requests.md` (append a CR), not direct edits.

### 6.2 FFI / JNI additions (all additive; owner A)

flow-ffi handle becomes `Box<enum Handle { Core(FlowCore), Router(Router) }>`; every
existing symbol keeps its signature and works on both kinds. Feature `remote =
["dep:flow-remote"]`; `flow-router` is always linked.

| Kotlin `@JvmStatic external` | C | Semantics |
|---|---|---|
| `nativeCapabilities(): String` | `char *flow_capabilities(void)` | `Capabilities` JSON |
| `nativeOpenRouter(routerConfigJson: String, localHandle: Long): Long` | `FlowHandle *flow_open_router(const char *json, FlowHandle *local)` | takes ownership of `local` (0/NULL = none; Kotlin must not unload it afterwards); 0 + last error (`no_engine_enabled`, `bad_config`) |
| `nativeUpdateRouter(h: Long, routerConfigJson: String): Boolean` | `int flow_update_router(FlowHandle*, const char*)` | `update_config` |
| `nativeProbeServer(serverConfigJson: String): String` | `char *flow_probe_server(const char*)` | blocking `HealthReport` JSON; never throws |
| `nativeRouteStatus(h: Long): String` | `char *flow_route_status(FlowHandle*)` | `RouteStatus` JSON |

Kotlin flow: existing `EnginePlan` loads local (`nativeLoadModels`, ASR-only retry) when
`CAP_LOCAL && allow_client`, then `nativeOpenRouter(cfg, localHandle)`; all session calls
go to the router handle. `FakeFlowCore` stays the mock-engine path.

### 6.3 Merge order (each step green on its own: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` at root; `cd apps/flow-windows && cargo test`; `./gradlew testDebugUnitTest assembleDebug` for all flavours)

| # | Branch | Owner | Content | Unblocks |
|---|---|---|---|---|
| 0 | `core-p1` | (other agent) | already in progress | everything (CommitResult fields) |
| 1 | `hybrid-contract` | S | flow-core additive change, `flow-protocol` + fixtures, `flow-router` with policy + fake-backend tests + `impl Backend for FlowCore`, router fixtures, `docs/protocol.md` | W and A switch from stubs to the real router (local-only works) |
| 2 | `hybrid-server` | S | `flow-remote`, `flow-server`, `flow-contract-tests` (equivalence, matrix, build matrix), systemd unit | real remote e2e |
| 3a | `hybrid-ffi-android` | A | flow-ffi additions, flavours, gates, dev page | — |
| 3b | `hybrid-windows` | W | features, router adapter, dev tab | — |

W and A start immediately against the §1.2 API with a local stub `Router` (W: a trait
object in their adapter; A: Kotlin fakes of the new natives) and rebase onto step 1 when
it lands; 3a/3b merge in either order after step 2. Step 1 must keep `Router` signatures
exactly as in §1.2; any change needs this doc updated first.

### 6.4 Definition of done (new `PROJECT_CONTEXT.md` "What works" rows)

1. `equivalence.rs` + `toggle_matrix.rs` green (mock), real-model equivalence green when models are present.
2. `flow-server` on the i7-6700 under systemd; Windows hybrid dictation via a VPN shows route `remote`, then `local` with `connect_failed` after `systemctl stop flow-server`, with no lost words.
3. Android `local` APK identical in behaviour to today (no-network gate green); `remote` APK has no ASR libs (gate green) and dictates through the server; `hybrid` falls back on airplane mode.
4. Developer page in both apps: toggles, host/port validation, Test connection with latency, active-engine row.

---

## Changelog for shells

Implementation notes for W and A against the §1.2 API (branch `hybrid-server`).
Every item is additive: code written against §1.2 compiles unchanged.

**2026-10-24 — D28: OpenRouter-only cloud, opt-in cloud cleanup (branch `feat/or-route` +
`feat/or-cloud`; spec `docs/proposals/openrouter-only-cloud.md`, flow-cloud API §4.1)**

Not additive for the cloud settings: the D18 provider list is replaced by one OpenRouter
config. Everything else (server/local routing, fixtures other than the cloud ones) is
unchanged.

*Rust API (flow-router / flow-core):*
* `RouterConfig.cloud` = `CloudConfig { schema: 2, openrouter: OpenRouterConfig { privacy:
  Privacy { zdr, data_collection }, stt: SttConfig { model, stt_path, language,
  send_dictionary, format_override, timeout_base_ms, consent_host, consent_at_ms }, cleanup:
  CleanupConfig { model, sort, timeout_ms (800-8000, default 2500), connect_timeout_ms,
  send_surrounding_text, send_screen_words, send_app_name, consent_host, consent_at_ms } },
  monthly_budget_usd }` (`fixtures/router_config_cloud.json`). `cloud.active` /
  `cloud.providers` are read only by the migration and never written.
* `RouterConfig.allow_cloud_cleanup: bool` and `cleanup_order: [CleanupRoute]`
  (`"cloud"`/`"local"`; empty = local LLM only, as before). Enable flow sets
  `["cloud","local"]`; `["local","cloud"]` = cloud only when no local LLM is loaded.
* `RouterConfig::migrate_d28() -> MigrationReport { migrated, key_from, key_to, delete_keys,
  removed, cloud_disabled, model_from, model_to, notice }`, idempotent; rules in
  `crates/flow-router/src/migrate.rs` (fixtures `fixtures/migration/<preset>.{v1,v2,report}.json`,
  all 7 old presets + a mixed case). `CloudUsage::migrate_d28(&report)` keeps the month's
  total (removed providers → one `removed` counter). The router also migrates any legacy
  config it is given (`new` / `update_config`) and then remembers the report
  (`cloud_notice()`, `ack_cloud_notice()`; restored usage is migrated too).
* Keys: one key, id `"openrouter"`, bound to `openrouter.ai`. `set_cloud_key(id, key)` ignores
  every other id except the migrated legacy OpenRouter id (alias). `set_cloud_key_for(&CloudProviderConfig)`
  is kept only for that legacy case. `with_cloud_key(|k| …)` lends the key to flow-cloud calls
  that are not dictations (catalogue account filter, key info).
* `probe_cloud(&OpenRouterConfig, key)` (was `&CloudProviderConfig`). `CloudFactory` takes
  `&OpenRouterConfig`. New `CleanupFactory` + `Router::with_cleanup_factory` (flow-cloud's
  `attach_with` installs both), `Router::set_cleanup(allow, order, cleanup)`,
  `Router::cleanup_status() -> CleanupStatus { supported, enabled, order, consent, blocked,
  circuit, model, timeout_ms, last: CleanUsage, month_cost_usd }` (`fixtures/cleanup_status.json`).
* `RouteStatus.cleanup_circuit`, `Failure::NoPrivateRoute` (`no_private_route`: no
  zero-retention provider serves the model; sticky until the model or privacy changes) and
  `Failure::CloudCleanupNotConfigured` (`cloud_cleanup_not_configured`). `ProviderUsage.purpose`
  `"stt"`/`"cleanup"`; `CloudUsage::cost_for(purpose)` for "Speech $x · Cleanup $y".
* `RouteInfo.llm_engine` (`local`/`cloud`/`server`), `llm_model`, `llm_cost_usd`;
  `CommitResult.llm` (`CleanUsage`: + `generation_id`, `ttft_ms`, `latency_ms`).
* `Backend::install_cleanup(Option<CleanupPlan>)` (default no-op): how the router puts
  `RoutedCleaner(cloud, local LLM)` into the local engine, the on-demand local engine and the
  cloud-STT backend. The server route never gets it.

*Routing rules (`crates/flow-router/src/gate.rs`, tests `tests/cleanup_policy.rs`):* C1 for
cleanup = build has the cleanup factory ∧ `allow_cloud_cleanup` ∧ `cloud` ∈ `cleanup_order` ∧
cleanup consent for openrouter.ai (STT consent does not count) ∧ key ∧ cleanup circuit closed ∧
`month_cost + estimate ≤ budget` (catalogue estimate with headroom; an unpriced model with a
budget waits for one real cost, then uses it × 2). Checked **per call**, so a consent revoked
or a toggle turned off mid-session never sends text. A refusal is a fast failure: with
`["cloud","local"]` the local LLM runs (≥ 800 ms left), else the deterministic text. Timeout
(`cleanup.timeout_ms`, 2500 ms) and guard rejection are final. Cost: exact `usage.cost`;
an estimate (flagged) when a generation exists without cost; nothing when no generation.
Circuit: auth / quota / `no_private_route` sticky until key or cleanup config change; others
back off 5 s doubling to 60 s. By default the prompt carries only the transcript, app
category, style and dictionary (`PromptScope::MINIMAL`); each `send_*` toggle adds its item.

*FFI (`apps/flow-android/JNI_CONTRACT.md` "OpenRouter (D28)", C `flow_core.h`):*
`nativeMigrateCloud(devJson)`, `nativeSetCleanup(h, json)`, `nativeCleanupStatus(h)`,
`nativeCloudNotice(h)` / `nativeAckCloudNotice(h)`, `nativeSetCloudModel(h, use, model)`,
`nativeSetCloudCacheDir(dir)`, `nativeCloudCatalog(h, use, query, filtersJson)`,
`nativeRefreshCloudCatalog(h, force)` (blocking), `nativeCloudKeyInfo(h)` (blocking).
`nativeCloudPresets` returns one OpenRouter entry. `nativeProbeCloud` / `nativeSetCloudKeyFor`
accept an `OpenRouterConfig` JSON. A legacy entry of another adapter is refused / ignored.

*Android shell work (W4, `apps/flow-android`):*
1. Startup, once: if the stored dev JSON has no `cloud.schema`, call `nativeMigrateCloud(devJson)`.
   Persist `config`. In `SecretVault`, move `flow_cloud_<report.key_from>` → `flow_cloud_openrouter`
   and delete `flow_cloud_<id>` for every `report.delete_keys`. Rewrite the persisted
   `cloud_usage` with the router's after `nativeSetCloudUsage`, then show `report.notice` once
   (or `nativeCloudNotice` + `nativeAckCloudNotice`). Test it with a Robolectric fixture from
   `crates/flow-router/fixtures/migration/`.
2. `data/CloudSettings.kt`: replace the provider list / `CloudPresets.ALL` / add-provider picker
   with the schema-2 data class (`cloud.openrouter.{privacy,stt,cleanup}`, `allow_cloud_cleanup`,
   `cleanup_order`). Keep `RouterJsonTest` in sync with `router_config_cloud.json` and
   `cleanup_status.json`.
3. `ui/CloudProviderActivity.kt` → `ui/OpenRouterActivity.kt`, one page with these parts:
   * Key: a `SecretField`, `nativeSetCloudKey(h,"openrouter",key)`, and "Check key", which calls
     `nativeCloudKeyInfo` off Main and shows label, limit / remaining, `usage_monthly`, and the
     `warn_no_limit` warning.
   * "Zero-retention providers only", with a confirm before turning it off.
   * A **Speech-to-text** card: opt-in plus the STT consent dialog, which records
     `stt.consent_*`; the model picker; language; send dictionary; and Test (`nativeProbeCloud`).
   * A **Text cleanup** card: opt-in plus its *own* consent dialog (spec §2.5 wording), which
     records `cleanup.consent_*`; the model picker; sort; timeout (800-8000); the three "Send …"
     toggles (off by default); and the order. Apply with `nativeSetCleanup`. Show the status
     from `nativeCleanupStatus`, with `blocked` as a hint.
   * Usage split: `cost_for("stt")` / `("cleanup")` from `nativeCloudUsage`, and the budget.
4. `ui/ModelPickerActivity.kt` (new): call `nativeSetCloudCacheDir(cacheDir)` once at app start.
   The list comes from `nativeCloudCatalog(h, use, query, filtersJson)` (filters `zdr_only`
   default on, `fast_only`, `free_only`, `stt_path`). "Refresh models" calls
   `nativeRefreshCloudCatalog` off Main. Never refresh in the background while both uses are
   off. Free-text ids go through `nativeSetCloudModel` (false + `bad_config` = invalid id).
5. Settings → AI cleanup row: "On-device / OpenRouter (<model>) / Off" plus the order.
6. Debug log (D21): add `llm_engine`, `llm_model`, `llm_cost_usd` (route) and `llm_ttft_ms`,
   `llm_generation_id` (`llm`). The `local` flavour exports the same symbols, which answer "not
   in this build". Rebuild the natives and run `tools/probes/check_native_so.sh` per flavour
   (the contract lists the new symbols).

*Windows shell work (W5, `apps/flow-windows`):*
1. `dev_settings.rs` / `prefs.rs`: on load, a legacy `cloud` runs
   `RouterConfig::migrate_d28()`. Persist the result. In `credentials.rs`, move
   `flow-clone/cloud/<key_from>` → `flow-clone/cloud/openrouter`, delete every
   `delete_keys` entry and keep the host binding `openrouter.ai`. Show `notice` once.
   `normalize_cloud` / `reconcile_consent` / `removed_providers` go: consent is per use, and
   the host is constant.
2. `app.rs` / `router_adapter.rs`: replace `set_cloud_key_for(&provider)` with
   `set_cloud_key("openrouter", key)` and `probe_cloud(&provider, key)` with
   `probe_cloud(&cfg.cloud.openrouter, key)`. `flow_cloud::attach_with` now also installs the
   cleanup factory. Remove `dev_cloud_presets` and `dev_cloud_new_id`; keep
   `dev_cloud_set_key/clear_key/usage/reset_usage`.
3. New Tauri commands:
   * `dev_or_catalog(use, query, filters)` → `flow_cloud::CatalogStore::search_json`. The store
     lives in `%LOCALAPPDATA%\FlowClone\cache\openrouter-catalog.json`; register it with
     `flow_cloud::catalog::set_default_store`.
   * `dev_or_refresh_catalog(force)` → `refresh`, inside `router.with_cloud_key`, on a blocking
     task.
   * `dev_or_key_info` → `flow_cloud::keyinfo::fetch`.
   * `dev_or_test_stt` → `probe_cloud`.
   * `dev_or_test_cleanup` → a sample sentence through `flow_cloud::cleaner_for`.
   * `dev_or_set_cleanup` → `Router::set_cleanup`.
   * `dev_or_cleanup_status` → `cleanup_status()`.
4. `ui/src/settings.ts` (+ committed `ui/dist`): the same page and semantics as Android step 3.
   The AI cleanup panel gains the engine choice (On-device / OpenRouter) and the order. The key
   field stays `type=password`.
5. Tests: `cloud_settings` (schema 2 round trip, separate consents), credential migration,
   and `cloud_integration`. For the last one, the mock is reached only through
   `flow_cloud::testing::override_base_url` (the `mock-server` feature, dev-dependency only),
   never through a base URL in config.

   *Done on `feat/or-windows` (W5):* the catalogue cache is
   `%APPDATA%\FlowClone\cache\openrouter-catalog.json` (next to prefs.json; user
   decision, not `%LOCALAPPDATA%`). Commands as listed plus `dev_or_get` (the page view),
   `dev_or_catalog_status`, `dev_or_set_model`, `dev_or_set_stt`, `dev_or_set_privacy`,
   `dev_or_set_budget`, `dev_or_ack_notice`; `dev_cloud_set_key(key)` /
   `dev_cloud_clear_key()` take no provider id; `dev_cloud_test` is `dev_or_test_stt`.
   Consent is recorded by the backend (host fixed to openrouter.ai, time of the call); the
   page only says the dialog was accepted. The Developer form carries no cloud fields.

**2026-10-21 — flow-protocol + flow-router landed**

* Types §1.2 left open, now concrete (`crates/flow-router/src/lib.rs`):
  * `StartInfo { queued: bool }`.
  * `BackendError { kind: Failure, message: String }`; `Failure` serialises to the `route.fallback` strings (`connect_failed`, `auth_failed`, `proto_mismatch`, `busy`, `timeout`, `disconnected`, `circuit_open`, `server_not_configured`, `local_not_ready`, plus `error` for a local engine failure).
  * `HealthReport` (JSON = `fixtures/health_*.json`): `ok`, `engine`, `error` (a `Failure` string), `message`, `server` ("host:port"), `rtt_ms {min,median,max}`, `server_version`, `server_core`, `postprocess_rev`, `postprocess_match`, `proto`, `asr_backend`, `asr_model`, `llm`, `workers`, `busy`, `checked_at_ms`.
  * `RouteStatus { last: Option<RouteInfo>, circuit: CircuitStatus { open, reason, retry_in_ms, until_config_change }, health: Option<HealthReport> }` (`fixtures/route_status.json`).
  * `RouterError` = `NoEngineEnabled | LocalNotReady | ServerNotConfigured | ServerUnavailable(String) | BadConfig(String)`, `kind()` = the §1.2 snake_case strings; `Display` = `"<kind>: <detail>"`.
* `Backend::session_failure(&self) -> Option<BackendError>` — new trait method **with a default** (`None`); shells' local wrappers need not implement it. The router polls it on every push to detect a dead remote session (busy, lost connection, no `started` in time).
* New `Router` methods: `with_local_capability(bool)` (builder: declare a local engine before its models are loaded, so `allow_client` with no model gives `local_not_ready`, not `no_engine_enabled`; default = a local backend was passed to `new`/`set_local`), `with_local_info(asr_engines, llm)` (fills `capabilities()`), `config()`, `probe_config(&ServerConfig)` (Test connection with the unsaved form; touches no router state), `reset_circuit()`.
* Validation: `RouterConfig::validate() -> Result<(), RouterError::BadConfig>` implements the §5 rules (host syntax via `flow_router::net::validate_host`, public host requires `tls`, pin 64 hex, token empty or ≥ 16, timeout ranges). `ServerConfig::addr()` gives "host:port" (IPv6 bracketed). `flow_router::net::classify(host)` is the §2.6 address class.
* flow-core: besides `CommitResult.route` / `RouteInfo` / `POSTPROCESS_REV`, `flow_core::VERSION` (crate version, used for `Capabilities.core_version` and `hello.core`).
* Behaviour decisions:
  * Server-only (`allow_client` off) and the server is unreachable at start: `start_session` returns `Err(server_unavailable: …)` (or `server_not_configured`) **and** a following `commit()` returns ERROR with `route.fallback` set, so either check works.
  * Both toggles off: `Err(no_engine_enabled)`; `commit()` returns ERROR `no_engine_enabled: …` (route `None`).
  * `commit()` with no session (never started / cancelled) returns ERROR `no active session` with `route: None`; every started session's result has `route`.
  * Circuit breaker: transient failures (`connect_failed`, `timeout`, `disconnected`) open it for 5 s doubling to 60 s, only consulted when a local fallback is ready; after the backoff the next session tries the server again (half-open; no background re-probe thread). `auth_failed`/`proto_mismatch` keep it open until the server config changes (also without a fallback). `busy` never opens it. A successful remote result or probe closes it.
  * Health is probed only by `probe_server()` / `probe_config()` (the shell calls it after Save and from Test connection); the router starts no background probes.
  * Fallback replay enlarges the local session's `ring_capacity_samples` to fit the buffer + 30 s, so no replayed sample is dropped.
* **Designed for a third route (opt-in cloud STT, planned; not implemented).**
  * `EngineKind`, `Prefer` and `Failure` are `#[non_exhaustive]`: shells must `match` with a `_` arm (a `Cloud` engine kind and cloud failures such as quota errors will be added).
  * `RouterConfig::route_order() -> Vec<EngineKind>` is the policy as an ordered list of allowed routes (today derived from `prefer` + the toggles). The cloud follow-up will add an `order: [..]` config field that supersedes `prefer` (which stays accepted as legacy JSON); shells that read the order through `route_order()` need no change.
  * `RouteInfo.provider: Option<String>` (flow-core, omitted from JSON when `None`) will name the cloud provider (`"openai"`, `"elevenlabs"`, `"openrouter"`, …).
  * Running the deterministic pipeline on an externally produced transcript already has a clean entry point: a cloud STT client is an `flow_core::asr::AsrEngine` (`transcribe(pcm, &AsrRequest) -> AsrOutput`, the request carries the dictionary/screen-word prompt and language), wrapped by `FlowCore::from_engines(asr, cleaner)`. That keeps VAD (no audio is sent for silence), the hallucination guard, spoken cleanup, the dictionary, the LLM step and post-processing on-device, so a cloud route is a third `Backend` built from a `FlowCore` and needs no flow-core change. TODO (cloud follow-up): if a provider returns text only after its own streaming endpointing, add `FlowCore::process_transcript(text, &SessionConfig, &FieldContext) -> CommitResult` for text-in/result-out without PCM.

**2026-10-22 — flow-remote, flow-server, contract tests landed**

* Use the remote backend with `Router::new(cfg, local, Some(flow_remote::factory("flow-windows 0.2.0")))`. Cargo feature `tls` on `flow-remote` adds `wss://` (rustls + Mozilla roots, or a pinned leaf: `tls_pin_sha256`); without it a `tls: true` config fails with `connect_failed`.
* Remote behaviour the shells will see:
  * `ws://` to a public host fails in `start_session`/`probe` with `connect_failed: plaintext to public address (enable TLS)` and opens no socket.
  * A warm connection is reused across sessions (idle close after 60 s), so only the first session after a pause pays the TCP + upgrade + hello round trips.
  * `started{queued:true}` (server busy) makes the router go local with `busy` when a ready local engine exists; without one the session waits on the server's queue (`queue_timeout_ms`, then ERROR).
  * Mid-session failure detection is by `session_failure()` on each push: connection closed, writer queue full (> 6.4 s backlog), server `busy`/`no_session`/`stale_sid`, or no `started` within `connect_timeout_ms`.
* `flow-server` (crate `crates/flow-server`): `flow-server [--config PATH] [--token-file PATH]`, `flow-server gen-token --out PATH` (43-char token, mode 0600, never overwrites), `flow-server check-config`. Features: `whisper` (default), `parakeet`, `llm`, `tls` (default). Example config and the systemd unit (with the kernel IP allowlist) are in `crates/flow-server/dist/`; the unit passes the token via `LoadCredential`. Not installed on any machine by this branch.
* Behaviour notes vs §3: auth failure is an HTTP 401 on the upgrade request (clients map 401/403 to `auth_failed`; the 4001 close code is reserved); a start while queued keeps the queued audio (≤ `max_samples`) and replays it into the worker with an enlarged ring; `max_session_s` overrun ends the session with `error too_large`; cancelling a queued session aborts its wait so the slot frees immediately.
* `tools/check-build-matrix.sh` (run by `cargo test -p flow-contract-tests --test build_matrix`) checks the flow-ffi and flow-windows flavours with `cargo tree`; the checks for features the shells have not added yet (`flow-ffi/remote`, `flow-windows/local`, `flow-windows/remote`) print SKIP. Strict mode (a SKIP fails) and cloud-strict mode are the defaults since the D28 merge; `CHECK_BUILD_MATRIX_STRICT=0` / `CHECK_BUILD_MATRIX_STRICT_CLOUD=0` relax them on a branch that has not caught up. Because Tauri itself depends on tokio, the Windows client-only check looks for the WS client stack (`flow-remote`, `tungstenite`, `axum`) rather than for tokio.

**2026-10-22 — live route + hands-free endpointing for the shells (branch `router-route`)**

Goal: the pill says "Transcribing via <route>…" **while recording** (and updates if the route
changes mid-session), and hands-free mode stops by itself on silence (D19). All additive;
pre-existing fixtures are byte-identical.

* **flow-router API** (`crates/flow-router/src/lib.rs`, tests `policy.rs` / `cloud_policy.rs`
  "live session route", fixtures `start_info_remote.json`, `poll_cloud.json`,
  `poll_fallback.json`, `poll_idle.json`):
  * `SessionRoute { engine: EngineKind, provider, model, server, fallback: Option<String> }`
    — JSON `{"engine":"local"|"remote"|"cloud","provider"?,"model"?,"server"?,"fallback"?}`
    (absent = omitted). `provider`/`model` (= `CloudProviderConfig.id` / `.model`) only for
    `cloud`; `server` ("host:port") only for `remote`; `fallback` = why the first route is not
    the one in use (the `route.fallback` strings). `SessionRoute::from_route_info(&RouteInfo)`
    is the same projection of a `CommitResult.route`.
  * `StartInfo` gains `route: Option<SessionRoute>` (backends leave it `None`; it lost `Copy`,
    and a struct literal needs `..Default::default()`; no shell builds one). It now has serde:
    `{"queued":bool,"route"?:…}`.
  * `Router::start_session_info(cfg) -> Result<StartInfo, RouterError>`: `start_session` that
    returns the start route (and `queued`: server busy, waiting for a worker).
    `start_session` is unchanged (it delegates).
  * `Router::poll()` now returns `RouterPoll { session: SessionPoll, route: Option<SessionRoute> }`
    (was `SessionPoll`). It `Deref`s to `SessionPoll` (`p.partial`, `p.endpoint_reached` still
    work) and `impl From<RouterPoll> for SessionPoll` (`.into()` / `.session` where a
    `SessionPoll` is needed). JSON = the `SessionPoll` fields flattened + `route`, so the idle
    JSON (no session) is byte-identical to `SessionPoll`'s.
  * `Router::session_route() -> Option<SessionRoute>` (just the route).
  * Semantics (contract tests): the start route is `Local` / `Remote{server}` /
    `Cloud{provider, model}`; a start-time skip is already in it (`fallback: "connect_failed"`,
    `"cloud_not_configured"`, …). After a mid-session switch (server dies → local replay; cloud
    dies → server) the next `poll().route` shows the new engine with `fallback` set (e.g.
    `{"engine":"local","fallback":"disconnected"}`). `route` is `None` with no session, after
    `commit`/`cancel`, after a failed start, and when every route failed (the commit is ERROR).
    Invariant: `SessionRoute::from_route_info(commit().route)` == the last polled route, and ==
    the start route unless a mid-session fallback happened.
* **flow-ffi**: C `char *flow_start_session_info(h, session_json)` (StartInfo JSON, NULL + the
  same last error/kind as `flow_start_session`) and `char *flow_poll(h)` (RouterPoll JSON; NULL
  for a NULL handle) in `flow_core.h`; JNI `nativeStartSessionInfo(h: Long, cfg: String): String?`
  (`(JLjava/lang/String;)Ljava/lang/String;`) and `nativePoll(h: Long): String` (`(J)Ljava/lang/String;`,
  never null; `0` = idle poll). Core handles answer too, without `route`. See
  `apps/flow-android/JNI_CONTRACT.md` "Live route + hands-free endpointing".

**What W must do** (flow-windows calls `flow_router::Router` directly, `router_adapter.rs`):

1. Start: replace `self.router.start_session(to_core_session(&cfg))` with
   `self.router.start_session_info(…)` and keep `info.route` (map the error exactly as today).
   Set the recording pill to the label of `info.route` right away (before the first partial).
2. Partial timer: replace `self.router.partial()` with one `let p = self.router.poll();` per tick.
   Use `p.partial` as today (`.filter(|s| !s.trim().is_empty())`), and when `p.route` differs
   from the shown one, relabel the pill (this is how "server died → local" shows up live).
   `p.route == None` mid-recording = the session is dead; keep the last label (the commit
   reports the error).
3. Labels (suggested): `local` → "Transcribing on this PC"; `remote` → "Transcribing via server"
   (+ `server` in a tooltip); `cloud` → "Transcribing via <provider display name> (<model>)",
   looking up `provider` in `RouterConfig.cloud.providers` for `display_name()`; with `fallback`
   set, append a short reason (e.g. "server unreachable" for `connect_failed`/`disconnected`/
   `timeout`, "cloud not set up" for `cloud_not_configured`). Match `EngineKind` with a `_` arm.
4. Hands-free (D19): in hands-free mode only, set `SessionConfig.endpoint_silence_ms =
   Some(pref)` (pref 300-10000 ms, default 2000; push-to-talk leaves it `None`). On the same
   partial tick, if `p.endpoint_reached` run the normal finish path (stop sound + `commit` off
   the UI thread + paste) exactly as when the user presses the hands-free key again; stop
   polling once finishing started. `endpoint_reached` is latched and the same on every route
   (computed in the router from the pushed audio, not by the backend).
5. The trait `CoreApi` is W's own; add a `poll()` (or `route()`) method there with a default
   so `StubCore` / `FlowCoreAdapter` keep compiling (`FlowCore::poll()` gives the
   `SessionPoll`; a bare core has no route — show "on this PC").

**What A must do** (Kotlin over JNI, `NativeFlowCore.kt`):

1. Declare `@JvmStatic external fun nativeStartSessionInfo(handle: Long, sessionConfigJson: String): String?`
   and `@JvmStatic external fun nativePoll(handle: Long): String`. (The libraries of every
   flavour export them; `check_native_so.sh` reads the list from JNI_CONTRACT.md, and the JNI
   stub in `tools/probes/jni_stub/` needs the two symbols added when Kotlin declares them.)
2. Start: call `nativeStartSessionInfo` instead of `nativeStartSession`; `null` = the same
   failure as `false` (read `nativeLastError` / `nativeLastErrorKind` as today). Parse
   `route` (org.json, all fields optional) and show it in the recording pill immediately.
3. Partial timer: replace `nativePartial` with `nativePoll` (one JNI call per tick); parse
   `partial`, `endpoint_reached`, `route`; relabel the pill when `route` changes; absent
   `route` while recording = dead session, keep the last label. Labels as for W (cloud: the
   provider's `label` from `DevSettings` for `route.provider`, plus `route.model`).
4. Hands-free: put `"endpoint_silence_ms": <pref>` in `SessionConfig` JSON only for hands-free
   sessions (`null`/omitted for push-to-talk; `CoreJson` should keep sending every other field).
   When a poll says `endpoint_reached: true`, run the normal finish path (stop sound,
   `nativeCommit` on `Dispatchers.Default`, paste) exactly as when the user stops; ignore
   further polls once finishing started.
5. Tests: `RouterJsonTest` can decode `poll_cloud.json`, `poll_fallback.json`, `poll_idle.json`,
   `start_info_remote.json` from `crates/flow-router/fixtures/`.

**2026-10-22 — review fixes, core/router/cloud/ffi side (branch `router-route`; review `docs/reviews/2026-10-22-integration.md` on `review-2026-10-22`)**

All additive at the API level (no signature removed); behaviour changes are listed so W and A
can adapt. Server-side items (S7-S10, C5, the server half of C4) are on branch `server-fixes`.

* **S1 Test provider / S4 key binding** (`flow-router`). A stored cloud key is bound to the saved
  provider's base-URL host + adapter. `update_config` with a changed host or adapter (or a removed
  provider) drops it; a key set by id before its provider is in the config binds to the first one
  saved. `probe_cloud(p, None)` uses the stored key only if `p` has exactly that host + adapter;
  otherwise `ok:false, error:"cloud_not_configured"`, message "enter the API key: the stored key is
  only sent to the saved host". New: `Router::set_cloud_key_for(&CloudProviderConfig, Option<SecretString>)`,
  C `flow_set_cloud_key_for(h, provider_json, key)`, JNI `nativeSetCloudKeyFor(h, providerJson, key): Boolean`.
  * W: on Save call `set_cloud_key_for(&saved_provider, key)`; when the host or adapter of a saved
    provider changes, delete its Credential Manager entry and ask for the key again (never re-send the
    old key, or it is bound to the new host). "Test provider" on an edited form must pass the typed key;
    show the message above when the field is empty.
  * A: same with `nativeSetCloudKeyFor` and the Keystore entry (`CloudSettings.edited`); the provider
    form's Test button passes the typed key when the host/adapter differ from the saved ones.
* **S2 URL parsing** (`flow-router`). `flow_router::cloud::parse_base_url(&str) -> Result<BaseUrl{https, host, port, ip}, String>`
  uses the `url` crate (reqwest's parser). It refuses `\`, whitespace / control characters anywhere,
  any userinfo (`user@`, `:pw@`), non-http(s) schemes and empty hosts; `CloudProviderConfig::host()`,
  consent and the http cleartext rule all use its host (WHATWG IPv4 forms like `0x7f.1` become
  dotted quads; percent escapes are decoded). **The Kotlin mirror (`CloudSettings.kt`) must match the
  vectors in `crates/flow-router/tests/url_parse.rs`** (the Android agent ports it). W: drop the
  hand-rolled host classifier in `dev_settings.rs` (~640-665) and call `parse_base_url` /
  `CloudProviderConfig::validate` instead.
* **S3 redirects** (`flow-cloud`). Provider calls never follow redirects (no Referer, https-only for
  https base URLs); a 3xx is `provider_error`. No shell change.
* **S5 mid-session re-check** (`flow-router`). Every fallback hop and replay re-checks the current
  toggles, `order`, consent and key; `update_config` / `set_cloud_key*` prune the live session's hops,
  and a live cloud session that is no longer allowed switches to the next allowed route at once (the
  pill changes on the next poll). A running server session keeps its route until the next session.
  No shell change (just call `update_config` as today).
* **S6 cleartext by resolved address** (`flow-remote`, `flow-cloud`, `flow_router::net`). `ws://`
  (and cloud `http://`) to a name resolves it and requires **every** address to be loopback, RFC 1918,
  ULA, link-local, or VPN (100.64/10, fd7a:115c:a1e0::/48) reached through an interface that holds
  a VPN address (`net::plaintext_ok_resolved`, `net::route_source`); the connection then goes to
  exactly those addresses. `*.local`, single-label and `*.ts.net` names still pass the config-time
  check (`ServerConfig::validate`) but no longer count as private by name. Choice/limit: carrier CGNAT
  also uses 100.64/10, so a phone on such a network with its VPN down cannot be told apart here;
  that case needs `wss://`. Failure: `connect_failed` "…: plaintext refused: … (enable TLS)"; shells
  may show it as "Use TLS for this server".
* **S11 stale preview** (`flow-core`). A partial decode of a cancelled/committed session can no
  longer appear in the next session; commit and cancel clear the preview. No shell change.
* **C2 blocking calls.** Documented in `flow_core.h` and JNI_CONTRACT.md "Threading". A: move
  `nativeStartSession*` off Main (it can block up to `connect_timeout_ms` on the server route).
  W: `start_session` blocks the same way; keep it off the UI/hook thread.
* **C3 failover off the audio thread** (`flow-router`). `push_audio` never connects: a mid-session
  failover runs on a helper thread while audio is buffered; `poll().route` names the route being
  switched to and `partial` is `None` until it is installed. No shell change.
* **C4 cancel during commit** (`flow-router`, `flow-core`, `flow-remote`, `flow-cloud`). `cancel()`
  with no open session aborts a `commit()` running on another thread: it returns at once with ERROR
  `cancelled` (route set), the engine is told to stop (local ASR abort flag, server `cancel`, cloud
  upload skipped if not started), and no fallback is tried. `start_session` no longer cancels a
  running commit (only the open session). W: Esc during processing (and the 35 s timeout) should call
  `cancel()` and treat an ERROR `cancelled` result as "dropped, nothing to paste". A: same for the
  20 s UI timeout, which already calls `nativeCancel`.

**2026-10-22 — remaining review fixes (branch `crates-final`; C6-C10, C18, S19, P3/P4)**

No API change; behaviour the shells can see:

* `Router::start_session[_info]` calls are serialised (C7). Two overlapping starts used to
  orphan the first session (and its server lease). Now the second one cancels the first, as a
  sequential start always did.
* A panic inside a backend while the router held a lock no longer wedges the router (C8). The
  session that was open is cancelled and dropped, and the next `start_session` works. The panic
  itself still reaches the caller (the FFI already catches it).
* Routing (C10): with `order` `[server, cloud]` and an open server circuit, a cloud route with
  no provider, consent, key or budget left no longer counts as a fallback, so the server is
  retried (half-open) instead of failing the dictation.
* `CommitResult.truncated` is now also `true` when a fallback replay lost audio past
  `max_samples` (C18). flow-core no longer counts samples past `max_samples` as
  `dropped_samples`, and `audio::resample_to_16k(_, 0)` returns no audio instead of panicking.
* flow-core (C6): a `start_session` that overlaps the previous `commit` (hands-free back to
  back) no longer changes that commit's context (app category, screen words) or its
  `dropped_samples`.
* Cloud, keyless self-hosted servers (P3): a `custom`-preset `openai_compat` provider whose base
  URL host is loopback, LAN or VPN is usable **without a key**. The request then carries no
  `Authorization` header, and **Test provider** works with an empty key field. Consent is still
  required. `has_cloud_key()` stays `false` for it. Shells may drop a "key required" check on
  Save for that case (W/A Developer forms). Named presets (Groq, OpenAI, …), public hosts and the other adapters still need a key, as does
  a provider whose id has a stored key bound to another host.
* AssemblyAI (S19): upload and create are not retried on a 504, so a timed-out create is never
  billed twice. A transcript still processing at the commit deadline is deleted in the
  background once it finishes (zero-retention). An upload whose create failed can't be deleted
  through the API and expires under AssemblyAI's retention policy.
* `tools/check-build-matrix.sh` also checks the Android (`aarch64-linux-android`) and Windows
  (`x86_64-pc-windows-msvc`) flavours with `--target` (P4).
* Self-hosting: `tools/server/install.sh` and `docs/server-setup.md`. The latter maps the
  Developer settings to the server's address, token and TLS pin.

**2026-10-23 — D22 pause-segmented decoding + latency fields (branch `feat/streaming-asr`)**

Shell steps: `docs/proposals/streaming-asr-shell-api.md`. All additive; no message, C or JNI
symbol changes.

* `SessionConfig.segment_pause_ms` (recommended 800) / `segment_min_ms` / `segment_soft_max_ms`
  travel in `start.config` like every other field. The server runs the unmodified core, so a
  server dictation is segmented exactly like a local one, and router fallback replay finds
  the same cuts (`flow-contract-tests/tests/segmented.rs`). A pre-D22 server ignores the field
  and returns the full text. The cloud backend never segments (`CloudAsr::segmentable` false;
  one upload at commit, C5).
* `HelloOk.features` (`["segments"]` from this server on; omitted = none). Router-internal; old
  clients ignore it (`fixtures/server_hello_ok_features.json`).
* `HelloOk.languages` (2026-10-30, D33): the language keys with their own server model
  (`["he"]`; omitted = none). A session with that forced language decodes on it
  (`fixtures/server_hello_ok_languages.json`, `docs/protocol.md` §3).
* `CommitResult.timings` gains `wait_ms`, `segments`, `pre_asr_ms` (omitted when 0;
  `fixtures/server_result_segmented.json`). The server's `result` log line now has
  `asr_audio_ms`, `wait_ms`, `segments`, `pre_asr_ms`.
* `RouteInfo.client_ms`: the router's own release → result wall time. For server results it
  includes the network and the server's queue, which the server's `total_ms` does not.
* Hands-free: `endpoint::RECOMMENDED_SILENCE_MS` is now 1500.

**2026-10-23 — D24 never-fail fallback (branch `fix/never-fail`)**

Live bug (S25+, hybrid, `keep_local_warm=false`, Parakeet imported, VPN relay):
`start_failed`, "Server unreachable (connect_failed: no handshake within 800 ms)". Root cause:
with keep-warm off the shell opened the router with no local backend, so after the server's
connect timeout `start_routes` found no ready local engine and returned
`Err(server_unavailable)`, although the client route was allowed and a model was imported.
800 ms was also too short for the relay (3+ round trips of 150-300 ms), so the server was
"down" while flow-server saw the connection arrive.

* **On-demand local engine** (`flow-router`): `LocalLoader = Arc<dyn Fn() -> Result<Arc<dyn
  Backend>, String>>`; `Router::with_local_loader(l)` / `set_local_loader(Option<l>)`. When the
  client route comes up with no warm engine (server down at start, open circuit, mid-session
  death, commit timeout), the session goes live on the client route immediately: audio is
  buffered (the C3 failover machinery), the loader runs on the failover thread, then the buffer
  is replayed. Client-first + cold: the server is tried first (`local_not_ready`), the load only
  happens if it fails. Load failure with no later route = ERROR `server_unavailable: <server
  error>; phone model failed to load: <why>`. A cancel during the load drops the session (no
  commit); the loaded engine stays for the next session. A cold loader counts as a viable
  fallback, so an open circuit skips the server without waiting (`circuit_open`).
* **Loading flag**: `StartInfo.loading`, `RouterPoll.loading` (omitted while false; fixtures
  `start_info_loading.json`, `poll_loading.json`), `Router::local_loading()`. Shells show
  "Loading phone model…" while it is true.
* **Debug fields**: `RouteDiag { connect_ms, local_cold, local_load_ms, local_load_error }` in
  `RouteStatus.last_diag` (fixture `route_status_diag.json`), recorded at commit. Android's D21
  line gains `connect_ms`, `local_cold`, `local_load_ms`.
* **Connect timeout**: default 800 → **2500 ms** (`flow_router::DEFAULT_CONNECT_TIMEOUT_MS`;
  `LEGACY_CONNECT_TIMEOUT_MS` = 800 for the shells' one-time migration; fixtures
  `router_config_{default,hybrid,local_only,cloud}.json` changed accordingly). Adaptive:
  `effective_connect_timeout_ms(cfg, rtt_median)` = `max(cfg, clamp(rtt × 6, 1500, 10000))`
  once a successful probe of this server measured its RTT; the remote backend is rebuilt when
  the effective value changes. Never lowers the configured value.
* **Pre-warm**: `Backend::warm()` (default `Ok(())`; flow-remote opens/keeps the warm WS) and
  `Router::prewarm() -> bool` (**blocking**: off the UI thread). Skipped with the server route
  off / unconfigured / circuit open / a live session; a failure opens the circuit, so the next
  dictation goes straight to local. Android calls it when a text field gets focus (≤ once per
  20 s); Windows does not yet (no focus signal; startup and Save already probe).
* **API note**: `StartInfo`, `RouterPoll`, `RouteStatus` gained fields; struct literals outside
  the crate need `..Default::default()` (the shells build none except flow-ffi's poll).
* **flow-ffi**: `json_api::set_local_loader(h, "[ModelConfig, …]")` (tried in order) and
  `json_api::prewarm(h)`; JNI `nativeSetLocalLoader(h, json): Boolean`, `nativePrewarm(h): Boolean`
  (JNI_CONTRACT.md). No C symbols added.
* **Android**: `EngineRoute.plan` keeps the Native plan as `onDemand` when keep-warm is off;
  `assemble` hands `EnginePlan.attempts(plan)` (selected engine ± LLM, then the whisper
  fallback ± LLM) to `nativeSetLocalLoader`. Errors name the missing piece ("— phone fallback
  unavailable: No speech model imported…", "… and the phone model failed to load (…)").
  Stored 800 ms migrates to 2500 once (`connect_timeout_default_2500`).
* **Windows**: same bug (`want_local` false → slot never loaded → "local model not ready").
  `RouterAdapter::set_on_demand(Some(ModelSpec))` installs a loader that `load_now`s the slot;
  `app.rs` re-syncs it at startup and on every prefs / Developer save. `prefs_rev` 2 migrates a
  stored 800 ms.

