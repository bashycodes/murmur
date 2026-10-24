# flow-server wire protocol v1 (normative)

Our own protocol between a hybrid client (`flow-remote`, reached through
`flow_router::Router`) and `flow-server` (D17). It is not Wispr's protocol
and is not derived from it. Types: `crates/flow-protocol/src/lib.rs`. Golden
fixtures: `crates/flow-protocol/fixtures/*.json` (one message per file,
`client_*` / `server_*`; `cargo test -p flow-protocol` round-trips each).
Design rationale: `docs/hybrid-architecture.md` §2.

## 1. Transport

| Item | Rule |
|---|---|
| Stream endpoint | `GET /v1/stream`, upgraded to WebSocket. The client offers subprotocol `flow.v1`; the server echoes it |
| Auth | `Authorization: Bearer <token>` on the upgrade request. No query-string token (it would land in access logs). Missing/wrong token → HTTP 401 before the upgrade (clients report `auth_failed`) |
| Health | `GET /v1/health` with the same auth → `200` + a `hello_ok` JSON body (without `"t"`) |
| Versioning | Path major `/v1/` changes only on a breaking change; the minor `proto` integer is negotiated in `hello` |
| Schemes | `ws://` only to loopback / RFC 1918 / 100.64.0.0/10 / fd7a:115c:a1e0::/48 / fc00::/7 / link-local / `*.ts.net` / `*.local` / single-label hosts (client-side check `flow_router::net::classify`); anything else needs `wss://` |
| TLS | rustls; self-signed leaf certificates may be pinned by SHA-256 of the leaf DER (`tls_pin_sha256`, 64 hex) |
| Sockets | TCP_NODELAY on both ends. WebSocket ping every 15 s; 45 s without any frame = dead |
| Sessions per connection | At most one active session. The client keeps an idle connection for 60 s for the next session |

## 2. Frames

* **Binary frame = audio** for the current session: raw PCM16 little-endian,
  mono, 16 kHz, even byte count, at most 65 536 bytes (recommended: 100 ms =
  3 200 bytes). No header. Audio while no session is active is dropped and
  counted, not an error. An odd-length frame → `error bad_request`; a frame
  over 64 KiB → `error too_large` (both non-fatal; the frame is dropped).
* **Text frame = JSON control message** tagged by `"t"`. Receivers ignore
  unknown fields. An unknown `"t"` or unparsable JSON → non-fatal
  `error bad_request`.

## 3. Messages

`<SessionConfig>`, `<FieldContext>` and `<CommitResult>` are flow-core's serde
types verbatim; every field has a default, so partial objects are valid and
additive core fields need no protocol change. The server never sets
`CommitResult.route` (the client's router does).

| Direction | Message | Meaning |
|---|---|---|
| C→S | `{"t":"hello","proto_min":1,"proto_max":1,"client":"flow-android 0.2.0","core":"0.1.0","postprocess_rev":"…"}` | First message on a connection |
| S→C | `{"t":"hello_ok","proto":1,"server":"flow-server 0.2.0+1a2b3c4","core":"0.1.0","postprocess_rev":"…","asr_backend":"parakeet","asr_model":"…","llm":false,"workers":1,"busy":0,"max_samples":19200000}` | Handshake accepted |
| C→S | `{"t":"start","sid":7,"config":<SessionConfig>}` | `start_session`. A `start` while a session is active cancels that session |
| S→C | `{"t":"started","sid":7,"queued":false}` | `queued:true` = no free worker; audio is buffered server-side until one frees |
| C→S | binary PCM | `push_audio` |
| C→S | `{"t":"context","sid":7,"ctx":<FieldContext>}` | `set_context` (any time before `commit`) |
| S→C | `{"t":"partial","sid":7,"text":"ask not"}` | Live preview, sent when it changes; only if `partial_interval_ms` is set |
| C→S | `{"t":"commit","sid":7}` | `commit` |
| S→C | `{"t":"result","sid":7,"result":<CommitResult>,"server_timings":{"queue_ms":0,"recv_audio_ms":11000}}` | Final answer for the session (any status, including `ERROR`) |
| C→S | `{"t":"cancel","sid":7}` | `cancel` |
| S→C | `{"t":"cancelled","sid":7}` | |
| C→S | `{"t":"ping","n":3}` | Application-level health ping |
| S→C | `{"t":"pong","n":3,"busy":0}` | `busy` = leased workers |
| S→C | `{"t":"error","sid":7,"code":"…","message":"…","fatal":false}` | `sid` omitted for connection-level errors. `fatal:true` = the server closes after sending |

* `sid`: client-chosen u64, increasing per connection. A control message
  whose `sid` is not the current session's → non-fatal `error stale_sid`.
* No dictionary or `learn_edit` sync: the dictionary travels in every
  `start.config`; `learn_edit` runs on the client. The server keeps no
  per-user state.
* `ModelConfig` (model paths, threads) is server configuration only.

### 3.1 Server limits

The server bounds every client-controlled size (`crates/flow-server/src/limits.rs`):

| Item | Limit |
|---|---|
| WebSocket message / frame | 128 KiB (larger → the connection closes) |
| `max_samples` | clamped to the server's `max_session_s` × 16 000 (advertised as `hello_ok.max_samples`) |
| `ring_capacity_samples` | ≤ that cap + 30 s replay headroom, else `too_large` |
| `llm_timeout_ms` | ≤ 120 000, else `too_large` |
| `partial_interval_ms` | raised to at least 100 |
| `context` | `before`/`selected`/`after`/`app_id`/`url_host` ≤ 8 KiB each; `screen_words` + `file_names` ≤ 1 000 entries, else `too_large` |

## 4. Error codes

| Code | When | Fatal |
|---|---|---|
| `unauthorized` | (HTTP 401 on upgrade; also WS close 4001 if ever sent after upgrade) | yes |
| `proto_mismatch` | `hello` range disjoint from the server's; followed by close 4002 | yes |
| `busy` | queue full at `start`, or `queue_timeout_ms` passed while queued; the session ends | no |
| `bad_request` | unparsable JSON, unknown `t`, odd-length audio, anything before `hello` (fatal then) | no / yes |
| `stale_sid` | control message for a session that is not current | no |
| `no_session` | `commit`/`context` with no active session | no |
| `too_large` | audio frame > 64 KiB, or session longer than the server's `max_session_s` (the session is cancelled), or a `start`/`context` over the server limits (§3.1; the message is ignored). Audio over the session's `max_samples` is not an error: the core sets `truncated` | no |
| `server_error` | anything else | no |

## 5. Version negotiation

`proto` = 1. The server speaks `[PROTO_MIN, PROTO_MAX]` = `[1, 1]` and picks
`min(client.proto_max, server.proto_max)` if that is ≥ both minimums
(`flow_protocol::negotiate`), else `proto_mismatch`. Additive changes (new
optional fields, new message types the peer may ignore, new error codes) do
not bump `proto`; bump it when a peer must understand something new; bump the
path to `/v2/` only for a breaking change. `core` / `postprocess_rev`
differences are not fatal; the client records `route.server_core` and the
Developer page warns. `proto` is deliberately independent of the product
version (D26, `docs/versioning.md`): `server` = `flow-server <version>+<commit>`
and `core` = the plain product version are informational only.

## 6. Client behaviour (flow-remote)

* New connection: TCP connect (+TLS) + upgrade + `hello`/`hello_ok`, all within
  `connect_timeout_ms`. A warm connection skips all of it.
* `start` is pipelined: the client sends `start` and audio immediately, without
  waiting for `started`. `started{queued:true}` or `error busy` marks the
  session busy; no `started` within `connect_timeout_ms` marks it lost.
* Audio goes through a bounded writer queue (64 frames ≈ 6.4 s); a full queue
  marks the session lost (the router then replays its own buffer locally).
* `commit` waits for `result` until the router's deadline
  `commit_timeout_base_ms + 0.3 × audio_ms + llm_timeout_ms`.

## 7. Server behaviour (flow-server)

* Runs an unmodified `FlowCore` session per `start`, fed the client's
  `SessionConfig` / `FieldContext`: VAD → ASR → hallucination guard → spoken
  cleanup → optional LLM → guard → post-processing → segments.
* Bind policy (S10): loopback, RFC 1918, 100.64/10, link-local and IPv6 ULA
  binds are always allowed; `0.0.0.0`, `::` or a public address only with
  `[tls]` or `--insecure-bind-any` (which logs a WARNING). Peers outside those
  private classes are dropped at accept unless `--insecure-bind-any`.
* Connection limits (S7): the TLS handshake and the HTTP request head must
  complete within `handshake_timeout_ms` (default 10 s; a silent socket is
  dropped too); at most `max_connections` (64) sockets in total and
  `max_connections_per_ip` (16) per peer are open, upgraded WebSockets
  included. Excess connections are closed at accept.
* Keepalive (C5): the server sends a WebSocket ping every `ws_ping_ms`
  (15 s) and drops a connection that has sent no frame at all, pongs
  included, for `ws_dead_ms` (45 s), with or without a session; its session
  ends and the worker is freed. A session that has not committed after
  `max_session_s` of wall-clock time ends with `error too_large`.
* `cancel` (or a replacing `start`, or a disconnect) while a commit is
  running is answered at once; the result is dropped and the server calls
  `FlowCore::cancel()` on the in-flight core (only until the job ends, so it
  never reaches the worker's next lessee). The worker returns to the pool
  when the core stops the job.
* A session leases a worker at `start` and returns it at `result`,
  `cancelled` or disconnect.
* Never written to disk: audio, transcripts, contexts, dictionaries. Logs
  carry sizes and timings only, unless `log_text = true`. This holds at any
  `RUST_LOG` level: events from dependencies (tungstenite, hyper, axum) are
  capped at INFO so frame and request dumps never reach the log.
