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
| `nativeVersion(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeVersion` | `(env: JNIEnv, _: JClass) -> jstring` | crate version, e.g. `"flow-ffi 0.1.0"`. Called once right after `loadLibrary` as a link check |
| `nativeLastError(): String?` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeLastError` | `(env, _: JClass) -> jstring` | Last error on this thread, or `null` |
| `nativeLoadModels(modelConfigJson: String): Long` | `(Ljava/lang/String;)J` | `Java_dev_flowclone_core_NativeFlowCore_nativeLoadModels` | `(env, _: JClass, cfg: JString) -> jlong` | `FlowCore::load_models(&ModelConfig)`. Returns the handle, or `0` with the last error set. Slow (model load) and called off the main thread |
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
| `nativeSetCloudKey(h: Long, providerId: String, key: String?): Boolean` | `(JLjava/lang/String;Ljava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKey` | `flow_set_cloud_key` | `Router::set_cloud_key`; `null` / `""` forgets it. Call after `nativeOpenRouter` for every stored provider and on Save / Delete. `false` + kind `error` for a core handle |
| `nativeSetCloudKeyFor(h: Long, providerJson: String, key: String?): Boolean` | `(JLjava/lang/String;Ljava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKeyFor` | `flow_set_cloud_key_for` | 2026-10-22, review S4: like `nativeSetCloudKey`, bound to exactly this `CloudProviderConfig` (base-URL host + adapter). Use it on Save with the provider being saved. `false` + `bad_config` (bad JSON) / `error` (core handle) |
| `nativeProbeCloud(h: Long, providerJson: String, key: String?): String` | `(JLjava/lang/String;Ljava/lang/String;)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeProbeCloud` | `flow_probe_cloud` | Blocking "Test provider": `HealthReport` (`engine:"cloud"`, `provider`, `model`, `transcript`, `latency_ms`, `cost_usd`) for an unsaved `CloudProviderConfig`; uploads the bundled 3 s JFK clip. `key` null = the stored key, **only** if `providerJson` has the host + adapter that key was saved for (review S1); a new or edited form must pass the typed key (else `ok:false, error:"cloud_not_configured"`, message "enter the API key…"). `h = 0` = temporary router (key required). Without `cloud`: `ok:false, error:"cloud_not_configured"` |
| `nativeCloudUsage(h: Long): String` | `(J)Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudUsage` | `flow_cloud_usage` | `CloudUsage` JSON `{month:"YYYY-MM", providers:[{provider_id, requests, billed_audio_ms, cost_usd, cost_estimated}]}`; Kotlin persists it after each commit |
| `nativeSetCloudUsage(h: Long, usageJson: String): Boolean` | `(JLjava/lang/String;)Z` | `Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudUsage` | `flow_set_cloud_usage` | Restore persisted counters after `nativeOpenRouter` (another month's are dropped). `false` + `bad_config` on bad JSON |
| `nativeResetCloudUsage(h: Long)` | `(J)V` | `Java_dev_flowclone_core_NativeFlowCore_nativeResetCloudUsage` | `flow_reset_cloud_usage` | Developer "Reset counters" |
| `nativeCloudCapabilities(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudCapabilities` | `flow_cloud_capabilities` | `{cloud, adapters}` of this build; must match `BuildConfig.CAP_CLOUD` (Developer page warns otherwise) |
| `nativeCloudPresets(): String` | `()Ljava/lang/String;` | `Java_dev_flowclone_core_NativeFlowCore_nativeCloudPresets` | `flow_cloud_presets` | `flow_cloud::presets()` (`CloudProviderConfig[]` without id / consent; `[]` without `cloud`) |

Existing natives are unchanged: `nativeOpenRouter` / `nativeUpdateRouter` accept `allow_cloud`,
`order` and `cloud` (a cloud-only config counts as an engine in a `cloud` build);
`nativeRouteStatus` adds `cloud_circuit`; `nativeCommit`'s `route` may be `engine:"cloud"` with
`provider`, `model`, `billed_audio_ms`, `cost_usd`, `cost_estimated`. New `route.fallback` values:
`cloud_not_configured`, `budget_exceeded`, `quota_exceeded`, `provider_error`. Host test:
`cargo test -p flow-ffi --features jni,cloud --test jni_cloud` (real JVM, real router, flow-cloud's
mock provider: no key → device, key → cloud, 401 → `auth_failed` + full replay, key never echoed).

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

## Threading: blocking vs non-blocking (review C2)

Kotlin must never call a **blocking** native on the main thread.

| Blocking (worker thread / `Dispatchers.IO`) | Non-blocking (any thread, incl. main and the AudioRecord thread) |
|---|---|
| `nativeLoadModels` (model load) | `nativePushAudio` (never waits; a mid-session failover connects on a native helper thread) |
| `nativeStartSession`, `nativeStartSessionInfo` (server route: TCP + TLS + WebSocket hello, up to `connect_timeout_ms`; local: may wait for the previous commit's worker) | `nativePoll`, `nativePartial`, `nativeSetContext` |
| `nativeCommit` (ASR + LLM; server/cloud round trip) | `nativeCancel` (also aborts a commit running on another thread, see below) |
| `nativeProbeServer`, `nativeProbeRouter`, `nativeProbeCloud` | `nativeRouteStatus`, `nativeCapabilities`, `nativeCloudCapabilities`, `nativeCloudPresets`, `nativeLastError*` |
| `nativeUnload` (joins the engine worker; waits for a running commit) | `nativeOpenRouter`, `nativeUpdateRouter`, `nativeResetCircuit` (no network) |
| | `nativeSetCloudKey`, `nativeSetCloudKeyFor`, `nativeCloudUsage`, `nativeSetCloudUsage`, `nativeResetCloudUsage` |

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
  address must be loopback / private / link-local, or tailnet (100.64/10, fd7a:115c:a1e0::/48)
  reached through a tailnet interface; otherwise `connect_failed` "plaintext refused" (use TLS).

## JSON payloads Kotlin sends (see `CoreJson.kt`, pinned by `CoreJsonTest`)

```jsonc
// ModelConfig
{"asr_backend":"whisper" /* or "parakeet" (asr_path = model dir) or {"mock":{"text":"…"}} */, "asr_path":"/data/user/0/dev.flowclone/files/models/ggml-base-q5_1.bin",
 "llm_path":null /* or ".../files/llm/qwen2.5-0.5b-instruct-q4_k_m.gguf" when AI cleanup is on */,
 "llm_lib":null /* default lookup; host tests pass a path */, "threads":4}
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
