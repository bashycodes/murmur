# Hybrid local/server architecture

Status: design (branch `hybrid-design`), decision D17 in `docs/DECISIONS.md`.
Covers the `core-p1` API (segments, `plain_text`, `empty_reason`, typed load errors,
`vad`/`hallucination_guard`/`spoken_cleanup`/`llm_prefill` session flags).

**Goal.** A shell (Windows, Android) dictates either on-device ("client", today's path)
or by streaming to a self-hosted `flow-server` on the user's own machine (i7-6700, 8
threads, 19 GB RAM, no GPU; LAN or Tailscale). Voice never reaches a third party.
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
    pub token: String, pub connect_timeout_ms: u64 /*800*/, pub commit_timeout_base_ms: u64 /*3000*/,
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

**Client-first** (`prefer=client`, both allowed): local is primary; remote is used only when
`local.ready()` is false at `start_session` (`fallback=local_not_ready`). Mid-session
local failures do not exist in practice (in-process), so there is no mid-session switch.

**Health.** `probe()` = connect + `hello` + 3 × `ping`/`pong`; reports min/median/max RTT.
Cached for `health_ttl_s`; refreshed on `update_config`, at app start, and after any
failure. Sessions do not wait for a probe; the circuit breaker uses the last outcome.

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
* Native sockets are not subject to Android's cleartext policy (Network Security Config covers Java stacks only), so `ws://` to a tailnet address works without `usesCleartextTraffic`. The router's own address check (§2.6) is the gate.
* Tailscale on Android is a VPN: no extra permission or code.

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
| `{"t":"hello","proto_min":1,"proto_max":1,"client":"flow-android 0.2.0","core":"0.1.0","postprocess_rev":"…"}` | `{"t":"hello_ok","proto":1,"server":"flow-server 0.1.0","core":"0.1.0","postprocess_rev":"…","asr_backend":"parakeet","asr_model":"sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8","llm":false,"workers":1,"busy":0,"max_samples":19200000}` | (handshake) |
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

* Shared bearer token: `flow-server gen-token` writes 32 random bytes base64url (43 chars) to `token_file` (mode 0600). Constant-time compare. Clients store it (Windows: DPAPI `CryptProtectData` blob in prefs; Android: Android Keystore AES-GCM key `flow_dev_token`, ciphertext in the `dev` prefs file). Minimum accepted length 16.
* The server refuses to start without a token unless `auth = "none"` **and** every bind address is loopback.

### 2.6 Transport security

| Server address (client-side check, `flow_router::net::classify`) | `ws://` | `wss://` |
|---|---|---|
| loopback, RFC 1918, 100.64.0.0/10 (Tailscale/CGNAT), fd7a:115c:a1e0::/48 (Tailscale), fc00::/7, link-local, `*.ts.net`, `*.local`, single-label names | allowed; Developer page shows "unencrypted: OK on Tailscale (WireGuard), LAN only otherwise" | allowed |
| anything else (public IP or public DNS name) | **refused** (`connect_failed: plaintext to public address`) | required |

Recommended: Tailscale (WireGuard already encrypts; bind the server to its 100.x address).
TLS (server `[tls] cert/key`, rustls) for LAN without Tailscale; self-signed certs pinned
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
| `src/main.rs` | CLI: `flow-server [--config PATH]`, `gen-token`, `check-config` |
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
token_file = "/etc/flow-server/token"           # required unless auth="none" on loopback
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
* `dist/flow-server.service`: `DynamicUser=yes`, `StateDirectory=flow-server`, `Environment=TMPDIR=/var/lib/flow-server/tmp` (Parakeet `bpe.vocab`), `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`, **`IPAddressDeny=any` + `IPAddressAllow=localhost 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 fd7a:115c:a1e0::/48 fe80::/10`** (the kernel enforces "no third party"), `MemoryMax=8G`, `Restart=on-failure`, `Nice=5`.

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
| `server.connect_timeout_ms` | number | 800 | 100-10000 |
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
* **Test connection** → `probe_server()` with the form's unsaved values, off the UI thread. Shows "OK · median 4 ms (3-6) · flow-server 0.1.0 · parakeet" or the error code with a hint (`auth_failed` → "token rejected", `connect_failed` → "is flow-server running / Tailscale up?", `proto_mismatch` → versions).
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
2. `flow-server` on the i7-6700 under systemd; Windows hybrid dictation via Tailscale shows route `remote`, then `local` with `connect_failed` after `systemctl stop flow-server`, with no lost words.
3. Android `local` APK identical in behaviour to today (no-network gate green); `remote` APK has no ASR libs (gate green) and dictates through the server; `hybrid` falls back on airplane mode.
4. Developer page in both apps: toggles, host/port validation, Test connection with latency, active-engine row.

---

## Changelog for shells

Implementation notes for W and A against the §1.2 API (branch `hybrid-server`).
Every item is additive: code written against §1.2 compiles unchanged.

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
