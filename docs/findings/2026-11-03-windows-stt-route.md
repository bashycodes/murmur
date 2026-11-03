# Which speech route the Windows PC uses, 2026-11-03

> **Status:** read-only investigation (code + server journal + the PC's settings and debug
> log). No settings changed, no app launched. Question: the server journal shows the PC's
> `flow-windows 1.0.0+…` client connecting several times since 1 Nov with the owner key, but
> no dictation finishes there. Which route does the PC use, and is that intended?

## Answer

* **Speech route: this PC (on-device), on purpose.** The PC's stored dictation order is
  `client, server, cloud` (cloud off). The router's `prefer` is the first of server/client
  (`DevSettings::prefer`), so every dictation starts on this PC. The server is only a
  fallback, used when the on-device model is not ready. All 15 dictations since 1 Nov went
  `route=local` with no fallback. The last server dictation was on 30 Oct, a
  `local_not_ready` fallback.
* **The server connections are health probes, not sessions.** Each one is
  `probe_in_background` → `Router::probe_server` → `RemoteBackend::probe`. It does a
  connect, hello, 3 pings and a close. The server logs `connected` at the hello and nothing
  more, because no session starts. They happen at app start and when Developer settings are
  saved. The 13:49:52Z connection is 1 s after the running process started (13:49:51Z). The
  13:30Z one follows the 1.0.0 install (the exe was written at 13:20Z). The five between
  06:30Z and 06:40Z were app restarts during the Vulkan laptop check
  (`2026-11-02-vulkan-he-laptop-check.md`). The Windows app never calls `Router::prewarm`,
  so there are no warm-up connections.
* **Matches the intended order: yes for speech.** D17's default is server first
  (`DevSettings::default` → `[Server, Client]`). The owner set this PC to "this PC first"
  before 30 Oct: every prefs backup (30 Oct, 31 Oct pre-0.12.0, 2 Nov pre-1.0.0) already has
  `client, server, cloud`. No migration reorders it (`migrate_d28` only moves cloud to the
  end). `2026-10-31-laptop-model-setup.md` also records "local first". This is the same
  situation as the phone in `2026-10-24-phone-local-only.md`. Meetings: `meeting_routes` is
  not stored, so D47 has meetings follow the dictation order (this PC first). The one meeting
  on 2 Nov was transcribed on this PC (22.9 s of audio in 13.6 s) with no server connection,
  as intended.
* **Not as intended: AI cleanup.** The PC's cleanup order is this PC (on) → server (on) →
  OpenRouter (off). This PC's model is not loaded (`llm_enabled: false`), and the server has
  an LLM. So by D36 the server should clean every on-device dictation, through a `clean`
  request. It never does. The server has logged 0 `clean`/`cleaned` lines from the PC, ever,
  against 113 from other clients. Every PC dictation since 31 Oct has an empty `llm`,
  `llm_engine` and `cleanup_route`. Cause, from the code: the Windows `LocalBackend`
  (`apps/flow-windows/src/router_adapter.rs`) does not implement `Backend::install_cleanup`
  or `llm_cleaner`. The trait default ignores the plan. So the router's D36 cleanup chain
  (server and cloud hops) never reaches the Windows on-device engine, which only ever uses its
  own LLM. The shared `flow_router::local` backend, used by Android, implements both. The
  `cleanup_order` tests on Windows cover settings and migration only, not a dictation being
  cleaned. The same gap should also block OpenRouter cleanup for on-device dictations when
  this PC's model is off. That is inferred from the code and not seen at runtime (cloud
  cleanup is off on the PC).

## Evidence (redacted)

### Server journal, 1–3 Nov (`flow_server::ws`, the PC's peer only)

| time (UTC) | event | client |
|---|---|---|
| 11-02 06:30:53, 06:36:02, 06:37:01, 06:38:54, 06:40:48 | `connected` key=owner | `flow-windows 1.0.0+3c8df49.dirty` |
| 11-02 13:30:21 | `connected` key=owner | `flow-windows 1.0.0+a24090a.dirty` |
| 11-02 13:49:52 | `connected` key=owner | `flow-windows 1.0.0+a24090a.dirty` |

* There are no `result`, `cancelled`, `session ended by disconnect`, `clean refused` or
  `cleaned` lines for the PC. `connected` is logged right after `hello_ok`
  (`crates/flow-server/src/ws.rs`), before any session, so a probe logs exactly one line.
* Since 20 Oct the PC has 40 `connected`, 3 `result` and 4 `cancelled`, all on 27–30 Oct.
* The PC made no connection at all during its 15 dictations (2 Nov, 21:14Z–21:31Z) or the
  meeting (21:23Z–21:24Z). An on-device dictation whose cleanup went to the server would have
  connected then.

### The PC

* Installed: Murmur 1.0.0 hybrid. The running `flow-windows.exe` started 2 Nov 13:49:51Z.
* `%APPDATA%\FlowClone\prefs.json` → `developer`:
  * `allow_server: true`, `allow_client: true`, `route_order: ["client","server","cloud"]`
  * `allow_cloud: false`
  * `cleanup_engines: local on, server on, cloud off`
  * `server: {host: <configured>, port: 7313, tls: false, connect_timeout_ms: 2500, health_ttl_s: 30}`
  * `token_dpapi: <redacted>`
* Top level: `cleanup_level: "medium"`, `llm_enabled: false`, `raw_mode: false`,
  `asr_engine: "parakeet"`, `hebrew_model: "zipformer"`. `meeting_routes` is absent.
* `debug\dictations.jsonl` (71 dictations, 272 events; only route fields read, no text):

| period | build | dictations | route | fallback | AI cleanup |
|---|---|---|---|---|---|
| 27 Oct | 0.6.0 | 6 | remote | `local_not_ready` | — |
| 30 Oct | 0.9.1 | 8 | 7 local, 1 remote (`local_not_ready`) | — | — |
| 31 Oct | 0.11.1 | 38 | local | none | none |
| 31 Oct | 0.12.0 | 4 | local | none | 1 `local` (model was on then), 3 none |
| 2 Nov | 1.0.0 | 15 | local | none | none (`llm` empty) |

* Events since 1 Nov: there is no `server` probe-failure event, so every probe succeeded.

### Code path (integration at `bec2e8f`)

* Route: `DevSettings::router_config_json` sends `prefer: "client"` when client comes
  before server in `route_order`. With both allowed, the router starts on the local backend
  and falls back to the server only on `local_not_ready` / an error (D24).
* Probes: `app.rs::probe_in_background` is called once when the router is built at startup,
  and after each Developer settings save. `RemoteBackend::probe` opens a fresh connection,
  pings 3 times and closes it. `Router::prewarm` (D24) has no caller in the Windows app.
* Cleanup: `Router::refresh_cleanup` builds `cleanup_plan`. It calls
  `b.install_cleanup(plan)` on every non-remote backend, and `LazyLocal::ensure` does the
  same after an on-demand load. `Backend::install_cleanup` defaults to a no-op
  (`crates/flow-router/src/lib.rs`). `flow_router::local`'s `FlowCore` backend overrides it
  (`self.set_cleaner(plan(self.cleaner()))`). Windows `LocalBackend` does not override it.
  `FlowCoreAdapter` builds its cleaner only from `LlmSpec`, which is `Some` only when
  `llm_enabled` is true.

## Proposed fix (ticket body)

**Title:** Windows: AI cleanup order (D36) skips the server and OpenRouter for dictations
transcribed on this PC

**What:** In `apps/flow-windows/src/router_adapter.rs`, implement
`Backend::install_cleanup` and `Backend::llm_cleaner` on `LocalBackend`, matching
`flow_router::local`. Store the plan in `FlowCoreAdapter`. Apply
`plan(own LLM cleaner)` with `FlowCore::set_cleaner` to the loaded core, and again to every
core the adapter loads or reloads later (model change, on-demand load, LLM toggled). A
reload must not drop the server/cloud hops. `llm_cleaner` returns the on-device LLM only
(Command Mode, `server_cleans`).

**Acceptance criteria:**

1. A Windows integration test (`tests/router_integration.rs` or `cleanup_order.rs`): this
   PC's LLM off, cleanup order this PC → server → cloud, and a fake remote advertising
   `clean`. A dictation on the local route sends one `clean` request, and the result shows
   `llm_engine = "server"`.
2. The same with the server off and OpenRouter cleanup on: the fake cloud cleaner is called.
3. This PC's LLM on and first: it cleans, and the server gets no `clean`. The D27
   already-clean skip still applies only to the on-device LLM (D43).
4. After a model reload (for example, switching the English engine), test 1 still cleans
   through the server.
5. On the PC, with the current prefs: a short English dictation shows a `cleaned` line from
   the PC's peer in the server journal, and the debug log has `llm_engine: "server"`.
6. The speech route is unchanged (this PC first). No new server connection opens for a
   dictation whose cleanup order has nothing available beyond this PC.

**Out of scope:** the PC's speech route order. It is the owner's choice and works as
designed.
