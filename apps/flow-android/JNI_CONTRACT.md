# JNI contract: `dev.flowclone.core.NativeFlowCore` ↔ `crates/flow-ffi`

The Kotlin side is `app/src/main/java/dev/flowclone/core/NativeFlowCore.kt`.
The Rust side must export exactly these symbols from `libflow_ffi.so`
(`crate-type = cdylib`, feature `jni`, built for `arm64-v8a` by `tools/build-native.sh --flavor local|hybrid|remote`,
which wraps `cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared … --features jni,whisper[,remote,cloud]` and adds
the NDK CMake toolchain and 16 KB page-size link flags). `libc++_shared.so` (needed by whisper.cpp)
ships next to it. Implemented in `crates/flow-ffi/src/jni_api.rs`; `tools/probes/check_native_so.sh`
checks the exported set against this table.

The library is loaded with `System.loadLibrary("flow_ffi")`. If it's missing, the app
still runs and shows an in-app message (`NativeFlowCore.loadLibrary()` returns
`Unavailable`).

## Conventions

- All methods are **static** natives on class `dev.flowclone.core.NativeFlowCore`.
  The second C parameter is therefore a `jclass` (`JClass` in jni-rs 0.21).
- The **handle** is an opaque `jlong`: `Box::into_raw(Box<FlowHandle>) as jlong`, where
  `FlowHandle` is a bare `FlowCore` (`nativeLoadModels`) or a `flow_router::Router`
  (`nativeOpenRouter`, D17). Every session function works on both kinds. `0` means
  "no handle / failure". After `nativeUnload(h)` Kotlin never uses `h` again, and
  `nativeUnload(h)` never runs while another call on `h` is in flight: every call holds a
  `HandleLease` (review C1), `unload()` only marks the handle dead, and the native free runs
  on a background thread once the last running call (push, poll, commit…) has returned.
  Other calls may come from several threads:
  `pushAudio` comes from the AudioRecord thread, `commit` from a worker coroutine,
  `setContext` from main. `FlowCore`'s methods are `&self` and internally synchronised.
- **Strings** are UTF-8 JSON. The schemas are the serde types in `flow-core::config`
  (`ModelConfig`, `SessionConfig`, `FieldContext`) and `flow-core::engine`
  (`CommitResult`), plus `learn::Candidate`. Kotlin always sends every field. Rust
  should still use `#[serde(default)]` (it already does).
- **Errors.** Functions that can fail return a sentinel (`0` / `false`) and store a message
  in a thread-local that `nativeLastError()` returns (and leaves in place). `commit`
  never fails at the JNI level: errors are reported as `{"status":"ERROR","error":"…"}`.
- **No panics across the boundary.** Wrap each body in `std::panic::catch_unwind`. On a
  panic, return the sentinel and set the last error. An unwound panic aborts the app process.
- **No Java exceptions are thrown from Rust.** Kotlin doesn't catch them on these paths.

## Functions

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | Rust (jni 0.21) | Semantics |
|---|---|---|---|---|
| `nativeVersion(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeVersion` | `(env: JNIEnv, _: JClass) -> jstring` | product version + commit (D26, `flow_build_info::FULL`), e.g. `"flow-ffi 0.2.0+1a2b3c4"` (`.dirty` suffix for a build from uncommitted changes). Called once right after `loadLibrary` as a link check |
| `nativeLastError(): String?` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeLastError` | `(env, _: JClass) -> jstring` | Last error on this thread, or `null` |
| `nativeLoadModels(modelConfigJson: String): Long` | `(Ljava/lang/String;)J` | `Java_dev_flowclone_core_NativeFlowCore_nativeLoadModels` | `(env, _: JClass, cfg: JString) -> jlong` | `FlowCore::load_models(&ModelConfig)`; with the shell-only field `"load_language": "he"` (D33/H5) `FlowCore::load_models_for(&cfg, Some(lang))`, which loads only the model that language routes to (the `languages` entry, else the default). Returns the handle, or `0` with the last error set. Slow (model load) and called off the main thread |
| `nativeUnload(handle: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeUnload` | `(env, _: JClass, h: jlong)` | Drop the box. `h == 0` is a no-op |
| `nativeStartSession(handle: Long, sessionConfigJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeStartSession` | `(env, _: JClass, h: jlong, cfg: JString) -> jboolean` | `start_session(SessionConfig)`. Any previous session is cancelled. `false` comes with the last error set |
| `nativePushAudio(handle: Long, pcm: ShortArray, length: Int): Boolean` | `(J[SI)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativePushAudio` | `(env, _: JClass, h: jlong, pcm: JShortArray, len: jint) -> jboolean` | `push_audio(&pcm[..len])`: 16 kHz mono PCM16, about 1600 samples (100 ms) per call. **Must not block**: copy with `get_short_array_region` into a stack/reused buffer, then push to the ring. Returns the core's bool (false = no session / samples dropped) |
| `nativeSetContext(handle: Long, fieldContextJson: String)` | `(JLjava/lang/String;)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetContext` | `(env, _: JClass, h: jlong, ctx: JString)` | `set_context(FieldContext)`. May arrive after audio started. Bad JSON is ignored (last error set) |
| `nativePartial(handle: Long): String?` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativePartial` | `(env, _: JClass, h: jlong) -> jstring` | `partial()`, or `null` |
| `nativeCommit(handle: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCommit` | `(env, _: JClass, h: jlong) -> jstring` | `commit()` serialised as `CommitResult` JSON. Blocks until ASR finishes. Kotlin calls it on `Dispatchers.Default` with a 20 s UI timeout, and then calls `nativeCancel` |
| `nativeCancel(handle: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeCancel` | `(env, _: JClass, h: jlong)` | `cancel()`. Idempotent |
| `nativeLearnEdit(handle: Long, pasted: String, edited: String): String` | `(JLjava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeLearnEdit` | `(env, _: JClass, h: jlong, pasted: JString, edited: JString) -> jstring` | JSON array `[{"original":…,"corrected":…}]`. Return `"[]"` on error |

## Router (D17, docs/hybrid-architecture.md §6.2) — additive

Since the hybrid work the app opens **one router handle** and sends every session call to it:
`nativeLoadModels` (local engine, when the flavour has one and the client route needs it) →
`nativeOpenRouter(cfg, localHandle)` → `nativeStartSession(router, …)` etc. `nativeUnload(router)`
frees the router and the local engine it owns.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | Semantics |
|---|---|---|---|
| `nativeLastErrorKind(): String?` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeLastErrorKind` | Stable kind of the last error on this thread (`model_not_found`, `llm_lib_not_found`, `load`, `bad_config`, `no_engine_enabled`, `local_not_ready`, `server_not_configured`, `server_unavailable`, `error`), or `null`. C: `flow_last_error_kind` |
| `nativeCapabilities(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCapabilities` | `Capabilities` JSON of this build (`local` = an ASR feature is built in, `remote` = feature `remote`). Must match `BuildConfig.CAP_LOCAL/CAP_REMOTE` (main screen error otherwise) |
| `nativeOpenRouter(routerConfigJson: String, localHandle: Long): Long` | `(Ljava/lang/String;J)J` | `Java_dev_flowclone_core_NativeFlowCore_nativeOpenRouter` | Takes ownership of `localHandle` (0 = none) **whatever the outcome**; Kotlin calls `NativeFlowCore.release()` first and never unloads it. Validates the config (`RouterConfig::validate`). `0` + kind `no_engine_enabled` (both routes off) / `bad_config` |
| `nativeUpdateRouter(h: Long, routerConfigJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeUpdateRouter` | `Router::update_config` (validated), applies from the next `start_session`; models are not reloaded. `false` + `bad_config` |
| `nativeProbeServer(serverConfigJson: String): String` | `(Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeProbeServer` | Blocking `HealthReport` JSON for an unsaved `ServerConfig` (Test connection; connect + hello + 3 pings). Never throws; without `remote`: `ok:false, error:"server_not_configured"` |
| `nativeProbeRouter(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeProbeRouter` | Blocking probe of the router's saved server; updates its circuit and `RouteStatus.health` (run after Save) |
| `nativeRouteStatus(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeRouteStatus` | `RouteStatus` JSON `{last: RouteInfo?, circuit: {open, reason, retry_in_ms, until_config_change}, health: HealthReport?}`; `{}` for a core handle |
| `nativeResetCircuit(h: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeResetCircuit` | Close the circuit breaker (Developer "Reset circuit"); no-op for a core handle |
| `nativeSetLocalLoader(h: Long, modelConfigsJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetLocalLoader` | D24 on-demand local engine: a JSON array of `ModelConfig`s (each may carry `load_language`, as `nativeLoadModels`) tried in order (`EnginePlan.attempts`); the router loads the first that works on its failover thread when a session needs the client route and none is warm (`keep_local_warm` off, server unreachable / circuit open). `[]` removes it. `false` + `bad_config` for bad JSON or a core handle. No network |
| `nativePrewarm(h: Long): Boolean` | `(J)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativePrewarm` | D24: open the server connection ahead of a dictation (**blocking** up to the connect timeout: off Main). Does nothing with the server route off / unconfigured / circuit open / a live session; a failure opens the circuit. `false` for a core handle |

Router errors: `nativeStartSession(router)` returns `false` with kind `no_engine_enabled` /
`local_not_ready` / `server_not_configured` / `server_unavailable` (server-only and
unreachable); `nativeCommit` then returns `ERROR` whose `error` starts with the kind.
Every started router session's `CommitResult` carries `route`
(`{"engine":"local"|"remote","fallback","server","rtt_ms","queue_ms","server_core"}`).
JSON shapes are pinned by `crates/flow-router/fixtures/*.json` (Kotlin: `RouterJsonTest`).
Libraries: `--features jni,remote` adds the WebSocket client (`flow-remote`, rustls for
`wss://`); the `local` flavour's library is built without it (no network code, I3).

## Cloud STT (D18, docs/cloud-stt.md) — additive

Third-party speech-to-text, opt-in from Developer settings. The `hybrid` / `remote` libraries are
built with feature `cloud` (`flow-cloud`: reqwest + rustls, webpki roots); the `local` library
is not (C2: `tools/check-build-matrix.sh`, `verifyLocal<BT>NoNetworkCode`). **Every flavour
exports these symbols**; without `cloud` they answer "not in this build". API keys enter native
code only through `nativeSetCloudKey` / `nativeProbeCloud` (never in router JSON); Rust keeps them
in a wiped-on-drop `SecretString` and never logs or returns them (C3).

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | C ABI | Semantics |
|---|---|---|---|---|
| `nativeSetCloudKey(h: Long, providerId: String, key: String?): Boolean` | `(JLjava/lang/String;Ljava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKey` | `flow_set_cloud_key` | `Router::set_cloud_key`; `null` / `""` forgets it. D28: `providerId` = `"openrouter"` (the migrated legacy OpenRouter id is accepted as an alias; any other id is ignored and returns `true`). Call after `nativeOpenRouter` and on Save / Delete. `false` + kind `error` for a core handle |
| `nativeSetCloudKeyFor(h: Long, providerJson: String, key: String?): Boolean` | `(JLjava/lang/String;Ljava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKeyFor` | `flow_set_cloud_key_for` | D28 (kept for ABI): `providerJson` = an `OpenRouterConfig` (or a legacy OpenRouter `CloudProviderConfig`) → sets the OpenRouter key; a legacy entry of another adapter is ignored (`true`). Prefer `nativeSetCloudKey(h,"openrouter",key)`. `false` + `bad_config` (bad JSON) / `error` (core handle) |
| `nativeProbeCloud(h: Long, providerJson: String, key: String?): String` | `(JLjava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeProbeCloud` | `flow_probe_cloud` | Blocking "Test provider": `HealthReport` (`engine:"cloud"`, `provider`, `model`, `transcript`, `latency_ms`, `cost_usd`) for an unsaved `OpenRouterConfig` (D28; the STT part is used; a legacy OpenRouter `CloudProviderConfig` is converted, any other adapter → `cloud_not_configured`, nothing sent); uploads the bundled 3 s JFK clip to openrouter.ai. `key` null = the stored key. `h = 0` = temporary router (key required). Without `cloud`: `ok:false, error:"cloud_not_configured"` |
| `nativeCloudUsage(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudUsage` | `flow_cloud_usage` | `CloudUsage` JSON `{month:"YYYY-MM", providers:[{provider_id, purpose?, requests, billed_audio_ms, cost_usd, cost_estimated}]}` (D28: `purpose` `"stt"` (also absent) / `"cleanup"`; one budget covers both); Kotlin persists it after each commit |
| `nativeSetCloudUsage(h: Long, usageJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudUsage` | `flow_set_cloud_usage` | Restore persisted counters after `nativeOpenRouter` (another month's are dropped). `false` + `bad_config` on bad JSON |
| `nativeResetCloudUsage(h: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeResetCloudUsage` | `flow_reset_cloud_usage` | Developer "Reset counters" |
| `nativeCloudCapabilities(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudCapabilities` | `flow_cloud_capabilities` | `{cloud, adapters}` of this build; must match `BuildConfig.CAP_CLOUD` (Developer page warns otherwise) |
| `nativeCloudPresets(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudPresets` | `flow_cloud_presets` | D28: exactly one entry (OpenRouter, id `openrouter`, default model); `[]` without `cloud`. Kept for ABI; the model list comes from `nativeCloudCatalog` |

Existing natives are unchanged: `nativeOpenRouter` / `nativeUpdateRouter` accept `allow_cloud`,
`order` and `cloud` (a cloud-only config counts as an engine in a `cloud` build);
`nativeRouteStatus` adds `cloud_circuit`; `nativeCommit`'s `route` may be `engine:"cloud"` with
`provider`, `model`, `billed_audio_ms`, `cost_usd`, `cost_estimated`. New `route.fallback` values:
`cloud_not_configured`, `budget_exceeded`, `quota_exceeded`, `provider_error`. Host test:
`cargo test -p flow-ffi --features jni,cloud --test jni_cloud` (real JVM, real router; D28: offline, see
"OpenRouter" below).

## OpenRouter (D28, docs/proposals/openrouter-only-cloud.md) — additive symbols

Kotlin uses all of them since branch `feat/or-android` (`NativeFlowCore`: `migrateCloud` at
startup via `FlowApp.migrateCloud` / `CloudMigration`, `setCloudCacheDir` before the router opens
and on the OpenRouter / picker pages, `cloudCatalog` + `refreshCloudCatalog` in
`ui/ModelPickerActivity`, `cloudKeyInfo` / `cleanupStatus` / `cloudNotice` / `ackCloudNotice` /
`setCloudModel` on the OpenRouter page and Developer status). Settings changes still go through
`nativeUpdateRouter` with the whole schema-2 config (the shell's prefs stay the source of truth),
so `nativeSetCleanup` is used only by the host test.

OpenRouter is the only cloud provider, for STT and for the opt-in text cleanup. The router
config's `cloud` is schema 2 (`cloud.openrouter.{privacy,stt,cleanup}`, plus `allow_cloud_cleanup`
and `cleanup_order` at the top level; `crates/flow-router/fixtures/router_config_cloud.json`). A
schema-1 config given to `nativeOpenRouter` / `nativeUpdateRouter` is migrated in memory, so a
removed provider is never used. The shell still runs `nativeMigrateCloud` once to persist the
result and fix its key store. Every flavour exports these symbols. Without `cloud`, catalogue /
key info answer `[]` / `{"error":"cloud_not_configured"}` and `nativeCleanupStatus` says
`supported:false`.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | C ABI | Semantics |
|---|---|---|---|---|
| `nativeMigrateCloud(devJson: String): String?` | `(Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeMigrateCloud` | `flow_migrate_cloud` | Pure, no handle. `{"config": RouterConfig, "report": {migrated, key_from, key_to, delete_keys[], removed[{id,name,preset,adapter}], cloud_disabled, model_from, model_to, notice}}`. Persist `config`, move `flow_cloud_<key_from>` → `flow_cloud_<key_to>`, delete `flow_cloud_<id>` for each `delete_keys`, show `notice` once. Idempotent (`migrated:false` for schema 2). `null` + `bad_config` on bad JSON. Fixtures: `crates/flow-router/fixtures/migration/` |
| `nativeSetCleanup(h: Long, cleanupJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCleanup` | `flow_set_cleanup` | Merge `{"allow_cloud_cleanup"?: bool, "cleanup_order"?: ["cloud","local"], "cleanup"?: CleanupConfig}` (missing = unchanged) and apply like `nativeUpdateRouter`. The consent dialog writes `cleanup.consent_host:"openrouter.ai"` + `consent_at_ms`. It is separate from STT consent. `false` + `bad_config` (bad JSON, timeout outside 800-8000, invalid model id, …) / `error` (core handle) |
| `nativeCleanupStatus(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCleanupStatus` | `flow_cleanup_status` | `CleanupStatus` `{supported, enabled, order, consent, blocked?, circuit?, model, timeout_ms, last?: {engine, model, cost_usd, cost_estimated, generation_id, ttft_ms, latency_ms}, month_cost_usd}` (`fixtures/cleanup_status.json`). `blocked` = why a call now would be refused (`cloud_cleanup_not_configured`, `circuit_open`, `auth_failed`, `quota_exceeded`, `no_private_route`). The budget is checked per call, not here |
| `nativeCloudNotice(h: Long): String?` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudNotice` | `flow_cloud_notice` | The one-time migration notice of a legacy config this router migrated, else `null` |
| `nativeAckCloudNotice(h: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeAckCloudNotice` | `flow_ack_cloud_notice` | The notice was shown; `nativeCloudNotice` returns `null` afterwards |
| `nativeSetCloudModel(h: Long, use: String, model: String): Boolean` | `(JLjava/lang/String;Ljava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudModel` | `flow_set_cloud_model` | `use` = `"stt"` / `"cleanup"`. Any id matching `^[a-z0-9._~/-]+(:[a-z]+)*$`, ≤ 128 chars (free text allowed, e.g. `:nitro`). Changing the model resets that use's sticky circuit. `false` + `bad_config` on an invalid id / use |
| `nativeSetCloudCacheDir(dir: String)` | `(Ljava/lang/String;)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudCacheDir` | `flow_set_cloud_cache_dir` | Call once at start with `cacheDir`. The catalogue cache is `<dir>/openrouter-catalog.json` (24 h TTL). No network |
| `nativeCloudCatalog(h: Long, use: String, query: String, filtersJson: String): String` | `(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudCatalog` | `flow_cloud_catalog` (no `h`) | Picker rows (JSON array: id, name, price label, badges, …; flow-cloud `PickerRow`) for `use`, searched by id/name. `filtersJson` `{zdr_only (default true), fast_only, free_only, stt_path}`. Served from memory, the cache or the baked snapshot; **no network**. `h` is unused (0 ok). `[]` for an unknown use / without `cloud` |
| `nativeRefreshCloudCatalog(h: Long, force: Boolean): String` | `(JZ)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeRefreshCloudCatalog` | `flow_refresh_cloud_catalog` | **Blocking** (≤ 10 s, off Main). Fetches `/models` + `/endpoints/zdr` (+ `/models/user` with the stored key) unless the cache is fresh or `force`. Returns `{source, fetched_at_ms, snapshot_date, models, stt, cleanup, error?}`; on `error` the previous catalogue stays. Sends no user data. Call it only from the OpenRouter page or while a cloud use is on |
| `nativeCloudKeyInfo(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudKeyInfo` | `flow_cloud_key_info` | **Blocking** "Check key": `GET /api/v1/key` with the stored key → `{label, limit, limit_remaining, limit_reset, usage, usage_daily, usage_weekly, usage_monthly, is_free_tier, free_requests_remaining, checked_at_ms, warn_no_limit}` (never the key), or `{"error": code}` (`cloud_not_configured` without a key / cloud, `auth_failed`, …) |

Behaviour (router, all flavours with `cloud`):
* Cloud cleanup runs only when the build supports it, `allow_cloud_cleanup` is on, `cloud` is
  in `cleanup_order`, cleanup consent and a key exist, its circuit is closed, and the budget
  allows the catalogue estimate. This is re-checked on **every** call, so revoking consent
  mid-dictation sends nothing. A refusal falls back to the on-device LLM (`["cloud","local"]`)
  or to the deterministic text; `fallback_reason` is then `llm_error: <code>`.
* Hard timeout `cleanup.timeout_ms` (default 2500): the commit returns the deterministic text
  (`llm_timeout`) and the stream is cancelled. The **server route never uses cloud cleanup**.
* By default only the transcript, the app category, the style and the dictionary leave the
  device. `send_surrounding_text` / `send_screen_words` / `send_app_name` add their item.
* `nativeCommit`'s `route` gains `llm_engine` (`local`/`cloud`/`server`), `llm_model`,
  `llm_cost_usd`. The result gains `llm` (`CleanUsage`). `nativeRouteStatus` gains
  `cleanup_circuit`. New `route.fallback` / `blocked` values: `no_private_route`,
  `cloud_cleanup_not_configured`.
* Host test: `cargo test -p flow-ffi --features jni,cloud --test jni_cloud` (real JVM, offline:
  migration, aliasing, catalogue, cleanup gate). The network paths are tested in
  `flow-contract-tests` against the OpenRouter mock.

## Live route + hands-free endpointing (2026-10-22) — additive

Lets the shell show "Transcribing via <route>…" **while recording** and run hands-free
endpointing (D19). Kotlin uses both since branch `android-final`: `NativeFlowCore.startSessionInfo` / `poll`
(`DictationController` polls every 250 ms while recording; the bubble preview shows
`RouteLabel` "via phone | server | <provider> (model)" and relabels on failover; a tap
(hands-free) session sends the user's "stop after silence" pref, default 2000 ms, push-to-talk never). Every flavour exports both; they work on core and router handles, but only
a router handle reports `route`. Existing natives are unchanged (`nativeStartSession` still
returns a `Boolean`; Kotlin may keep using it for push-to-talk).

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | C ABI | Semantics |
|---|---|---|---|---|
| `nativeStartSessionInfo(handle: Long, sessionConfigJson: String): String?` | `(JLjava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeStartSessionInfo` | `flow_start_session_info` | Same as `nativeStartSession`, but returns `StartInfo` JSON `{"queued":bool,"route"?:SessionRoute}` (fixture `start_info_remote.json`), or `null` with the same last error / kind as `nativeStartSession`'s `false`. `queued` = the server accepted but has no free worker yet |
| `nativePoll(handle: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativePoll` | `flow_poll` | Non-blocking `RouterPoll` JSON `{"partial":str\|null,"endpoint_reached":bool,"speech_ms":n,"trailing_silence_ms":n,"route"?:SessionRoute}` (fixtures `poll_*.json`). Never `null` (handle `0` = idle poll). Call it on the partial timer instead of `nativePartial`. `route` is present while a session is live and **changes** when a mid-session failure switches route (server dies → local replay; cloud error → next route); absent with no session or when every route failed (the commit will be `ERROR`) |

`SessionRoute` = `{"engine":"local"|"remote"|"cloud","provider"?,"model"?,"server"?,"fallback"?}`:
`provider` / `model` only for `cloud` (`CloudProviderConfig.id` / `.model`), `server`
(`"host:port"`) only for `remote`, `fallback` = why the first route is not the one in use (the
`route.fallback` strings). Absent fields are omitted. After the commit,
`CommitResult.route` projected the same way equals the last polled `route`.

Hands-free: send `"endpoint_silence_ms": <pref, 300-10000, default 2000>` in `SessionConfig`
**only** for hands-free sessions (omit / `null` for push-to-talk), poll with `nativePoll` on the
partial timer, and when `endpoint_reached` is `true` run the normal finish path (stop sound,
`nativeCommit`, paste) exactly as if the user had ended the session. `endpoint_reached` latches;
it is always `false` without `endpoint_silence_ms`.

## Command Mode (2026-10-28) — additive

Selected-text transforms (Windows Command Mode, `crates/flow-core/src/command.rs`): the shell
records a raw (`raw_mode: true`) instruction with the normal session calls, then asks the
router to apply it to the selection. The prompt, the answer sanitiser and the privacy gate are
the shared Rust ones, so Android and Windows behave alike. Every flavour exports it.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | C ABI | Semantics |
|---|---|---|---|---|
| `nativeCommand(h: Long, instruction: String, selected: String?): String` | `(JLjava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCommand` | `flow_command` | **Blocking** (LLM call, ≤ `command::DEFAULT_TIMEOUT` 25 s, plus a D24 on-demand model load when nothing else can answer). `Router::command`: the warm local engine's on-device LLM and/or OpenRouter cleanup per `cleanup_order`, consent, key, budget. A **selection** goes to OpenRouter only with `cloud.openrouter.cleanup.send_surrounding_text`; otherwise the on-device part answers, or `selection_not_allowed`. `selected` null / blank = write at the cursor. Never null: `{"ok":true,"text":…}` (sanitised, ready to replace the selection) or `{"ok":false,"kind":…,"error":…}` with `kind` ∈ `no_instruction`, `selection_too_long` (> 4000 chars), `no_llm`, `selection_not_allowed`, `timeout`, `empty`, `backend`; `error` is user-facing text. A core handle uses its own LLM only |

## Benchmark (2026-10-31) — additive

On-device benchmark hook for the debug-only `BenchActivity` (`tools/probes/android_bench`). Every
flavour's library exports it; only debug builds call it. No network, and no text but the
request's own.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | C ABI | Semantics |
|---|---|---|---|---|
| `nativeBenchClean(h: Long, requestJson: String): String` | `(JLjava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeBenchClean` | — | **Blocking** (one LLM call). The request is `{"mode":"prod"\|"v4c","input","context":FieldContext,"session":SessionConfig}`, run on a **core** handle's on-device LLM. `prod` runs `flow_core::refinish` on `input` (the product's text path) and returns `output`, `status` and `fallback_reason`. `v4c` sends the raw transcript with prompt v4c to the LLM once, with no guard. Both return `llm_raw`, `llm_error`, `llm_ms`, `llm_calls`, `wall_ms` and `llm_info`. Never null; on failure, or for a router handle, it returns `{"error":…}` |

## Admin API (D39, "Users & keys") — additive, `hybrid` / `remote` only

The owner manages the server's named keys from Settings → Your server → Users & keys. A
separate class, **`dev.flowclone.admin.NativeAdmin`** (Kotlin `object` + `@JvmStatic external`,
`app/src/main/java/dev/flowclone/admin/NativeAdmin.kt`), so the second C parameter is again a
`jclass`. Rust: `crates/flow-ffi/src/jni_admin.rs` over `admin_api.rs` (feature **`admin`** =
`remote` + `flow_remote::admin::AdminClient`); `tools/build-native.sh` enables it for `hybrid`
and `remote`, so the `local` library has none of these symbols (Kotlin treats
`UnsatisfiedLinkError` as "not in this build"). `serverConfigJson` is the saved
`ServerConfig` (as for `nativeProbeServer`). All four are **blocking** (one HTTP request,
≤ 15 s) and never return null. Answer: `{"ok":true,"value":…}` or
`{"ok":false,"code":…,"message":…}` with `code` from `flow_protocol::admin::code`
(`forbidden` = not an admin key: Kotlin hides the page). Nothing is logged; the Rust copy of
an answer is wiped after it is handed to Java.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | Semantics |
|---|---|---|---|
| `nativeAdminWhoami(serverConfigJson: String): String` | `(Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_admin_NativeAdmin_nativeAdminWhoami` | `GET /v1/whoami` → `value` = `{id, name, admin}` |
| `nativeAdminListKeys(serverConfigJson: String): String` | `(Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_admin_NativeAdmin_nativeAdminListKeys` | `GET /v1/admin/keys` → `value` = `{keys:[KeyView]}` (usage incl. `downloads`, `download_bytes`; `current` = this key) |
| `nativeAdminCreateKey(serverConfigJson: String, name: String, admin: Boolean): String` | `(Ljava/lang/String;Ljava/lang/String;Z)Ljava/lang/String;` | `Java_dev_flowclone_admin_NativeAdmin_nativeAdminCreateKey` | `POST /v1/admin/keys` → `value` = `{key, secret}`. The **secret** is shown once and never stored or logged by the app |
| `nativeAdminRevokeKey(serverConfigJson: String, id: String): String` | `(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_admin_NativeAdmin_nativeAdminRevokeKey` | `DELETE /v1/admin/keys/{id}` → `value` = `{key}`; `last_admin` for the last admin key |

## Model library (D41, 2026-10-31) — additive, `hybrid` / `remote` only

Model downloads from the set server's library (D40), wrapping `flow_remote::library` like the
Windows app. The natives live on their own class, **`dev.flowclone.server.ServerLibraryNative`**
(`object` + `@JvmStatic external`), which only the `hybrid` / `remote` source sets compile; the
library exports them only with cargo feature `library` (`tools/build-native.sh --flavor hybrid|remote`).
The `local` APK has neither the class nor the symbols nor any network code (D9). Both calls are
**blocking** (worker thread). Results are never null; a failure is
`{"ok":false,"error":<kind>,"message":…}` with `kind` ∈ `bad_config`, `not_configured`,
`network`, `auth`, `not_found`, `unavailable`, `protocol`, `mismatch`, `io`, `cancelled`. The key
never appears in a result.

| Kotlin (`@JvmStatic external`) | JNI descriptor | C symbol | Semantics |
|---|---|---|---|
| `nativeList(server: String): String` | `(Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_server_ServerLibraryNative_nativeList` | `server` = `ServerConfig` JSON (`DevSettings.toServerConfigJson`). `GET /v1/models` → `{"ok":true,"models":[{id,title,kind,language?,catalogue_id?,engine?,files:[{name,size,sha256}]}]}` (`flow_protocol::library`; entries with unsafe ids / names / hashes dropped) |
| `nativeDownload(server: String, modelId: String, file: String, dest: String, part: String, listener: Any?): String` | `(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;)Ljava/lang/String;` | `Java_dev_flowclone_server_ServerLibraryNative_nativeDownload` | One file (`file` = `{name,size,sha256}` JSON: the built-in pin for a catalogue copy, the listing for a custom model) into `dest` through `part` → `{"ok":true}`. Resumable: a network error / stall / short body keeps `part` and the next call sends `Range` + `If-Range`; `cancelled` and `mismatch` delete it; `dest` appears only verified, and a `dest` already holding the expected bytes is kept without a request. `listener` (nullable): `onProgress(Long)` = bytes of this file on disk (≤ 4/s), `isCancelled(): Boolean` polled between chunks; a listener method that throws is cleared (a throwing `isCancelled` counts as false) |

## Threading: blocking vs non-blocking (review C2)

Kotlin must never call a **blocking** native on the main thread.

| Blocking (worker thread / `Dispatchers.IO`) | Non-blocking (any thread, incl. main and the AudioRecord thread) |
|---|---|
| `nativeLoadModels` (model load) | `nativePushAudio` (never waits; a mid-session failover connects on a native helper thread) |
| `nativeStartSession`, `nativeStartSessionInfo` (server route: TCP + TLS + WebSocket hello, up to `connect_timeout_ms`; local: may wait for the previous commit's worker) | `nativePoll`, `nativePartial`, `nativeSetContext` |
| `nativeCommit` (ASR + LLM; server/cloud round trip) | `nativeCancel` (also aborts a commit running on another thread, see below) |
| `nativeProbeServer`, `nativeProbeRouter`, `nativeProbeCloud`, `nativePrewarm` (D24) | `nativeRouteStatus`, `nativeCapabilities`, `nativeCloudCapabilities`, `nativeCloudPresets`, `nativeLastError*` |
| `nativeUnload` (joins the engine worker; waits for a running commit) | `nativeOpenRouter`, `nativeUpdateRouter`, `nativeResetCircuit`, `nativeSetLocalLoader` (no network; the load itself runs later on a router thread) |
| `nativeRefreshCloudCatalog`, `nativeCloudKeyInfo` (D28, network), `nativeCommand` (LLM), `nativeBenchClean` (LLM) | `nativeSetCloudKey`, `nativeSetCloudKeyFor`, `nativeCloudUsage`, `nativeSetCloudUsage`, `nativeResetCloudUsage`, `nativeMigrateCloud`, `nativeSetCleanup`, `nativeCleanupStatus`, `nativeCloudNotice`, `nativeAckCloudNotice`, `nativeSetCloudModel`, `nativeSetCloudCacheDir`, `nativeCloudCatalog` |

Session semantics (2026-10-22, review fixes):

* `nativeCancel(h)` with no open session aborts a `nativeCommit` running on another thread:
  that commit returns at once with `{"status":"ERROR","error":"cancelled"…}`, the engine is
  told to stop (local ASR aborted, server sent `cancel`), and no fallback route is tried.
  `nativeStartSession*` only cancels the open session, never a commit still running for the
  previous dictation (hands-free back-to-back).
* A mid-session failover (server dies → next route) runs off the calling thread: while it
  connects, `nativePoll`'s `route` already names the route being switched to and `partial`
  is `null`; all audio pushed meanwhile reaches the new route.
* `nativeUpdateRouter` / `nativeSetCloudKey*` re-check the live session: fallback hops the new
  config no longer allows (toggle off, cloud not in `order`, consent revoked, key removed or
  re-bound) are dropped, and a live **cloud** session that is no longer allowed switches to the
  next allowed route immediately (a running server session keeps its route until the next
  session).
* Cloud keys are bound to the provider's base-URL host + adapter: a config that changes either
  (or removes the provider) drops the key in native code. Kotlin must also delete the stored
  key in that case and ask for it again; it must not re-send the old key for the new host.
* Base URLs are parsed with the `url` crate (reqwest's parser): `\`, whitespace, control
  characters and any userinfo are refused (`bad_config`). `CloudSettings.kt`'s mirror must
  give the same answers as `crates/flow-router/tests/url_parse.rs`.
* `ws://` and `http://` (cloud) to a **name** are decided by its resolved addresses: every
  address must be loopback / private / link-local, or a VPN address (100.64/10, fd7a:115c:a1e0::/48)
  reached through a VPN interface; otherwise `connect_failed` "plaintext refused" (use TLS).

## JSON payloads Kotlin sends (see `CoreJson.kt`, pinned by `CoreJsonTest`)

```jsonc
// ModelConfig
{"asr_backend":"whisper" /* or "parakeet" (asr_path = model dir) or {"mock":{"text":"…"}} */, "asr_path":"/data/user/0/dev.flowclone/files/models/ggml-base-q5_1.bin",
 "llm_path":null /* or ".../files/llm/qwen2.5-0.5b-instruct-q4_k_m.gguf" when AI cleanup is on */,
 "llm_lib":null /* default lookup; host tests pass a path */, "threads":4}
// ModelConfig while the dictation language routes to a Hebrew model (H5): only that model loads
{"asr_backend":"whisper", "asr_path":"…/ggml-base-q5_1.bin", "llm_path":null, "llm_lib":null, "threads":4,
 "languages":{"he":{"asr_backend":"sherpa_online" /* or "parakeet" */, "asr_path":"…/files/hebrew/zipformer-he"}},
 "load_language":"he"}
// SessionConfig
{"languages":["en"], "dictionary":[{"word":"wispr","replacement":"Wispr","isSnippet":false}],
 "styles":{"personal":"formal","work":"formal","email":"formal","other":"formal"},
 "cleanup_level":"none", "raw_mode":false, "max_samples":4800000, "platform":"android",
 "partial_interval_ms":null, "llm_timeout_ms":1500,
 "vad":true, "hallucination_guard":true, "spoken_cleanup":true /* user pref */, "llm_prefill":true}
// FieldContext
{"app_id":"com.Slack","url_host":"","app_category":"work_chat","before":"…≤500","selected":"","after":"…≤500","screen_words":[]}
```

`CommitResult` as Kotlin parses it:
`{"status":"FORMATTED|RAW_TRANSCRIPT|EMPTY|ERROR","asr_text","final_text","language"?,
"timings":{"audio_ms","asr_ms","llm_ms","postprocess_ms","total_ms","dropped_samples"},
"press_enter","fallback_reason"?,"truncated","error"?, "segments":[{"text"}|{"key":"Enter|Tab|Break"}],
"plain_text","empty_reason"?, "route"?}`. Kotlin pastes `segments` (not `final_text` +
`press_enter`), stores `plain_text` in history, and shows `empty_reason`. If the status is
unknown or the JSON is malformed, Kotlin treats the result as `ERROR`.

## Rust skeleton

```rust
#[cfg(feature = "jni")]
mod jni_api {
    use jni::{JNIEnv, objects::{JClass, JString, JShortArray}, sys::{jlong, jboolean, jint, jstring}};
    #[no_mangle]
    pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeLoadModels(
        mut env: JNIEnv, _c: JClass, cfg: JString) -> jlong { /* catch_unwind, serde_json::from_str, Box::into_raw */ }
    // … one fn per row above
}
```

Android-specific notes for the Rust side:
- `threads`: the shell sends `min(4, availableProcessors/2)`. Don't call
  `default_threads()` on Android, because big.LITTLE makes `available_parallelism` misleading.
- The model path always points to app-private storage (`filesDir/models/`, which the
  SAF import screen copies into). The core needs no Android storage APIs.
- LLM plugin: with `llm_path` set, `libflow_ffi.so` must be built with feature `llm`
  (`tools/build-native.sh --whisper --llm`), and `libflow_llm.so` ships next to it. Kotlin
  calls `System.loadLibrary("flow_llm")` before `nativeLoadModels`, so the core's bare-name
  `dlopen("libflow_llm.so")` resolves to the already-loaded library (jniLibs aren't
  extracted to disk, so there is no path to pass). A missing plugin fails `load_models`;
  the shell then reloads ASR-only.
- Network (D17/D18): only the `hybrid` / `remote` flavours have `INTERNET`, and only their
  library is built with `remote` and `cloud`. The `local` APK has no network permission
  (`verifyLocal<BT>NoNetworkPermissions`) and no WebSocket/TLS code in `libflow_ffi.so`
  (`verifyLocal<BT>NoNetworkCode`).
- Parakeet (`asr_backend:"parakeet"`): Kotlin sets `TMPDIR` to `cacheDir` before
  `nativeLoadModels` (hotword `bpe.vocab`); `libsherpa-onnx-c-api.so` + `libonnxruntime.so`
  ship next to `libflow_ffi.so` and resolve from the app's lib dir.
