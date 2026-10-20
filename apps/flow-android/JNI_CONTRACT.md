# JNI contract: `dev.flowclone.core.NativeFlowCore` ↔ `crates/flow-ffi`

The Kotlin side is `app/src/main/java/dev/flowclone/core/NativeFlowCore.kt`.
The Rust side must export exactly these symbols from `libflow_ffi.so`
(`crate-type = cdylib`, feature `jni`, built for `arm64-v8a` by `tools/build-native.sh --whisper`,
which wraps `cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared … --features jni,whisper` and adds
the NDK CMake toolchain and 16 KB page-size link flags). `libc++_shared.so` (needed by whisper.cpp)
ships next to it. Implemented in `crates/flow-ffi/src/jni_api.rs`; `tools/probes/check_native_so.sh`
checks the exported set against this table.

The library is loaded with `System.loadLibrary("flow_ffi")`. If it's missing, the app
still runs and shows an in-app message (`NativeFlowCore.loadLibrary()` returns
`Unavailable`).

## Conventions

- All methods are **static** natives on class `dev.flowclone.core.NativeFlowCore`.
  The second C parameter is therefore a `jclass` (`JClass` in jni-rs 0.21).
- The **handle** is an opaque `jlong`: `Box::into_raw(Box<FlowCore>) as jlong`. `0` means
  "no handle / failure". After `nativeUnload(h)` Kotlin never uses `h` again. It
  serialises `unload` against itself. Other calls may come from several threads:
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

## JSON payloads Kotlin sends (see `CoreJson.kt`, pinned by `CoreJsonTest`)

```jsonc
// ModelConfig
{"asr_backend":"whisper" /* or {"mock":{"text":"…"}} */, "asr_path":"/data/user/0/dev.flowclone/files/models/ggml-base-q5_1.bin",
 "llm_path":null, "threads":4}
// SessionConfig
{"languages":["en"], "dictionary":[{"word":"wispr","replacement":"Wispr","isSnippet":false}],
 "styles":{"personal":"formal","work":"formal","email":"formal","other":"formal"},
 "cleanup_level":"none", "raw_mode":false, "max_samples":4800000, "platform":"android",
 "partial_interval_ms":null, "llm_timeout_ms":1500}
// FieldContext
{"app_id":"com.Slack","url_host":"","app_category":"work_chat","before":"…≤500","selected":"","after":"…≤500","screen_words":[]}
```

`CommitResult` as Kotlin parses it:
`{"status":"FORMATTED|RAW_TRANSCRIPT|EMPTY|ERROR","asr_text","final_text","language"?,
"timings":{"audio_ms","asr_ms","llm_ms","postprocess_ms","total_ms","dropped_samples"},
"press_enter","fallback_reason"?,"truncated","error"?}`. If the status is unknown or the
JSON is malformed, Kotlin treats the result as `ERROR`.

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
- No network: the APK has no `INTERNET` permission (enforced by the
  `verifyNoNetworkPermissions` Gradle task), so any socket the core opens will fail.
