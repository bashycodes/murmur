# Review: `integration` @ 49b21c5 vs `master` @ 6160d43 (2026-10-22)

Scope: the whole codebase.
Axes: privacy/security, then correctness (router, lifecycle, FFI/JNI), then
shell edge cases, test gaps and simplification. Specs: `docs/DECISIONS.md`
(D17-D20), `docs/hybrid-architecture.md`, `docs/cloud-stt.md`,
`docs/protocol.md`, `docs/core-architecture.md`, and the research clone specs.

Labels: **CONFIRMED** = the failing path was traced end to end in the code (or
by a quick check). **PLAUSIBLE** = the code allows it, but it depends on
runtime or third-party behaviour that wasn't exercised. Paths are relative to
the repo root.

## Checked and sound (no action)

* **Server auth** (flow-server):
  * The bearer token is checked before the WebSocket upgrade, using a
    constant-time compare.
  * Short or empty tokens are refused (`MIN_TOKEN_LEN`), and `auth=none`
    requires a loopback bind.
  * The token is never logged.
  * Transcripts reach the log only when `log_text` is set, and the server
    warns when it is.
* **No audio or text on disk** on the server, router or core. The replay
  buffer is capped at `max_samples` and dropped with the session.
* **Cloud consent gate** (`crates/flow-router/src/router.rs:132-161`, 703-712):
  * Checked in Rust at session start and on every fallback hop.
  * Bound to the provider host; a `base_url` host change voids it.
  * No cloud route without both `allow_cloud` and an explicit `order` entry.
* **Key and token storage:**
  * `SecretString` redacts in Debug/Display and zeroes on drop.
  * Android stores cloud keys with Keystore AES-GCM. `allowBackup=false`, and
    `data_extraction_rules.xml` excludes every domain.
  * Windows stores cloud keys in Credential Manager and the server token as a
    DPAPI blob.
* **Client-only builds:** Windows `local,app` and the Android ffi
  `jni,whisper,llm,parakeet` build pull in no reqwest, rustls, tungstenite or
  flow-cloud (`tools/check-build-matrix.sh` passes).
* **C ABI:**
  * Every `extern` entry point is wrapped in `catch_unwind`.
  * Null and non-UTF-8 inputs are handled.
  * Strings are freed through `flow_string_free` → `CString::from_raw`.
* **Windows paste** sets the clipboard-history, cloud-clipboard and monitor
  opt-out formats. CSP is strict, and the capabilities grant no shell, fs or
  http.

## P1: privacy and security

| # | Status | Where | Failure scenario | Fix |
|---|---|---|---|---|
| S1 | CONFIRMED | `crates/flow-router/src/router.rs:577-580` (`probe_cloud`), `apps/flow-windows/src/app.rs:919-942`, `apps/flow-android/.../ui/CloudProviderActivity.kt:190-201` | "Test provider" with no key typed in falls back to the stored key for `p.id`, but takes the host from the unsaved form. Edit Groq's base URL to another host, press Test, and the saved `gsk_` key goes there with no consent check. | Use the stored key only when `p.host()` equals the saved provider's consent host; otherwise require a key in the form. |
| S2 | CONFIRMED (our parse) / PLAUSIBLE (reqwest side) | `crates/flow-router/src/cloud.rs:282-299` (`split_url`), Kotlin twin `data/CloudSettings.kt:200-211` | The authority is split only on `/?#`. For `http://evil.com\.local/v1` the parsed host is `evil.com\.local`, which classifies as LAN, so plain http and the consent host both pass. WHATWG/`url` treats `\` as `/` in special schemes, so the request goes to `evil.com` in cleartext with the key and the audio. | Parse with the `url` crate (the same parser reqwest uses) and validate `Url::host()`; reject `\`, whitespace and control characters. |
| S3 | CONFIRMED | `crates/flow-cloud/src/http.rs:89-95` (no redirect policy) | reqwest follows up to 10 redirects. On a cross-host redirect it strips only `Authorization`/`Cookie`, so ElevenLabs' `xi-api-key` header goes with it, even when downgraded to http. A 307/308 re-posts the audio body to a host the user never consented to. The Referer leaks Deepgram's query string, which contains dictionary terms. | `.redirect(Policy::none()).referer(false)`, plus `https_only(true)` for https base URLs. Add a mock 302/307 test per adapter. |
| S4 | CONFIRMED | `crates/flow-router/src/router.rs:303-319`, `apps/flow-windows/src/dev_settings.rs:339-360`, Android `CloudSettings.edited` | Keys are stored per provider id, not per host. A host change clears consent but keeps the key. After the user re-consents (the dialog only talks about audio), the old provider's key goes to the new host. | Clear the key on a host change, or store `(host, key)` and refuse to use it on a mismatch. |
| S5 | CONFIRMED | `crates/flow-router/src/router.rs:773-780` vs `switch_next` 988-1048; `update_config` 285-297 | Fallback hops are resolved at session start and never rechecked. Turn off Allow cloud, revoke consent or delete the key mid-dictation, then the server dies: the buffered audio is still replayed to the cloud. The same applies to Allow server. | Re-run `routes()` and `cloud_gate()` in `switch_next` before each hop, or prune the active session's hops in `update_config`/`set_cloud_key`. |
| S6 | CONFIRMED (no resolved-IP check) / PLAUSIBLE (exploit) | `crates/flow-router/src/net.rs:324-352`, `crates/flow-remote/src/conn.rs:184`, `apps/flow-windows/src/dev_settings.rs:640-665` | "ws:// refused on public addresses" is decided from the hostname: single-label names, `*.local` and `*.ts.net` count as private, and 100.64/10 counts as tailnet. A DNS search suffix or NXDOMAIN hijack, Tailscale Funnel (`*.ts.net` resolving publicly), DNS rebinding, or carrier-grade NAT on cellular with Tailscale down can all send the bearer token and the audio in cleartext to a non-private peer. | Resolve in flow-remote, require every resolved IP to pass `classify_ip().plaintext_ok()`, and connect to that `SocketAddr`. Stop treating `.ts.net` names as private; allow 100.64/10 only when a Tailscale interface is up. |
| S7 | CONFIRMED | `crates/flow-server/src/lib.rs:192-196`, 174-204 | The hyper `auto::Builder` has no `.timer()`, so the default header-read timeout is off. The TLS `accept` has no timeout and there is no connection cap. Anyone who can reach the port, without a token, can hold unlimited sockets (slowloris / file-descriptor exhaustion). | `.timer(TokioTimer::new())` with `header_read_timeout(10s)`, `timeout(10s, acc.accept(..))`, and global plus per-IP connection semaphores. |
| S8 | CONFIRMED | `crates/flow-server/src/ws.rs:491`, 593-596 → `crates/flow-core/src/engine.rs:406` | `ring_capacity_samples` and `max_samples` are taken from the client's `Start.config` without a clamp. A token holder (or a buggy client) sends `1<<34`, and `RingBuffer::new` aborts or OOM-kills the whole server. | Clamp in `SessionConfig` validation: ring ≤ 60 s, `max_samples` ≤ `max_session_s * 16k`. |
| S9 | CONFIRMED | `crates/flow-server/src/main.rs:762-768` | `RUST_LOG` overrides the config, and the `tracing-log` bridge is on. tungstenite logs every received text frame at trace (`"Received message {message}"`), so `RUST_LOG=trace` writes the dictionary, context, partials and results to journald regardless of `log_text`. | Force `tungstenite=off,tokio_tungstenite=off` in the filter; warn or refuse at trace. Extend `no_persist.rs` to run at trace. |
| S10 | CONFIRMED (design gap) | `crates/flow-server/src/config.rs:90-98`, 134-149; `lib.rs:226` | The server binary accepts a `0.0.0.0` or public bind over plain ws://, and there is no peer-IP filter. "LAN/Tailscale only" (D17) depends on the systemd `IPAddressAllow`, which is lost under docker, `cargo run` or a manual start. | Refuse an unspecified or public bind without `[tls]` or an explicit override; check `peer.ip()` against the same private classes before the upgrade. |
| S11 | CONFIRMED | `crates/flow-core/src/engine.rs:408-411` vs 700-717 | `start_session` clears the partial and resets `stop_partial=false` on the caller's thread. A partial decode of the previous session still running on the worker then completes and writes the old text into the new session. `cancel`/`commit` never clear the partial. A cancelled dictation's text reappears in the next live preview, possibly in another app. | Tag partials with the session id and drop mismatches; clear the partial on commit/cancel; use a per-session abort flag. |
| S12 | CONFIRMED (spec-faithful) | `apps/flow-android/.../insert/V3Inserter.kt:50` (+101/159/180/188/133/193), `session/DictationController.kt:182-189` | `isOurPackage` accepts whichever package is focused at commit time. Only `inputType` is rechecked; the banking denylist, user denylist, `isPassword` and data-sensitive checks are not. Dictate in Notes, switch to a banking app's memo field within the 20 s processing window (or press paste-retry later), and the text lands there. | Pin the start package and field; at commit and on retry re-run `BubbleVisibilityPolicy.evaluate`, and on a mismatch copy instead of inserting. |
| S13 | CONFIRMED | `apps/flow-windows/src/controller.rs:442-461`, `app.rs:404-411` | No foreground-window snapshot is taken at start. Alt+Tab while processing (up to 35 s), and the transcript is pasted into the other chat. | Record the HWND (and PID) in `BeginSession`; if it differs at paste time, leave the text on the clipboard and show a pill. |
| S14 | CONFIRMED | `apps/flow-windows/src/paste/win.rs:201-209` (`copy_text`) | Copy-last and History→Copy set only `CF_UNICODETEXT`, so transcripts go into Win+V history and to Microsoft's cloud clipboard sync. | Set `CanUploadToCloudClipboard=0` (and, behind a setting, `CanIncludeInClipboardHistory=0`). |
| S15 | CONFIRMED | `apps/flow-windows/src/controller.rs:326-338`, `app.rs:387-416`, `paste/win.rs:227-260` | Pastes are not serialized. Two paste-last presses within 500 ms: the second snapshots the first's transcript as "the user's clipboard" and restores it, and the user's real clipboard is lost. | Take a process-wide mutex in `paste::win::paste` and `copy_text`. |
| S16 | CONFIRMED | `apps/flow-android/.../insert/InsertionCoordinator.kt:37`, `service/FlowService.kt:351-381` | On any failure, including `EXCLUDED_FIELD`, the transcript is copied to the clipboard and never cleared, even with history retention set to "never". | Don't copy on `EXCLUDED_FIELD`; `clearPrimaryClip()` after about 60 s if the clip is still ours. |
| S17 | CONFIRMED (low) | `apps/flow-windows/src/history.rs:151-176`, 247-256; `apps/flow-android/.../data/Stores.kt:154-183` | History is plaintext SQLite with WAL. Delete/clear/"never" leave the rows readable in free pages and the `-wal` file. | `PRAGMA secure_delete=ON` and `wal_checkpoint(TRUNCATE)` (plus VACUUM) after a clear or a retention change. |
| S18 | CONFIRMED (low) | `apps/flow-windows/src/app.rs:684` (`dev_view`), `capabilities/default.json` | The decrypted server token is returned to the webview, against the design note. The overlay window can invoke every command, including `history_list` and `dev_cloud_*`. | Return a masked "token set" flag; scope the overlay window to `overlay_action`. |
| S19 | PLAUSIBLE | `crates/flow-cloud/src/adapters/assemblyai.rs:70-130`, `http.rs:180-185` | Upload succeeds, then create fails: the upload is never deleted. On a poll timeout, DELETE on a still-processing transcript is likely rejected. A 504 retry can re-upload or re-bill. This undercuts the zero-retention claim. | Delete the upload on create failure; delete after completion in the background; don't retry create on 504. |

## P2: correctness (router, lifecycle, threading, FFI/JNI)

| # | Status | Where | Failure scenario | Fix |
|---|---|---|---|---|
| C1 | CONFIRMED | `apps/flow-android/.../core/NativeFlowCore.kt:70-89`, `crates/flow-ffi/src/jni_api.rs:46-50`, 131-141 | Use-after-free. `unload()` is `@Synchronized` only against itself; `handle` isn't `@Volatile`, and `pushAudio`/`commit`/`partial` call native code without a lock. Settings save → `reload()` (`SettingsActivity.kt:112`), `ACTION_RELOAD_ENGINE`, or `onTrimMemory` after the 20 s commit timeout can drop the box while `nativeCommit`/`nativePushAudio` is running. | Make the native handle an `Arc` (each call clones it; unload drops one ref), or wrap every native call in a read lock and take the write lock in `unload`. |
| C2 | CONFIRMED | `FlowService.kt:89,112` → `DictationController.start` (Main) → `router.rs:397` → `crates/flow-remote/src/lib.rs:141-149` | In the hybrid/remote flavours, the TCP+TLS+WS handshake runs on the UI thread (ANR). `router.rs:399-402` also holds the `state` lock across that connect, and `session` is None meanwhile, so the first pushes are dropped (spec I5). | Start the session off Main; resolve the backend under the lock, connect outside it, or install a buffering `Active` first. |
| C3 | CONFIRMED (path) | `crates/flow-router/src/router.rs:430-433` → `switch_next` 1017 | A mid-session failover inside `push_audio` connects synchronously on the capture thread (up to `connect_timeout_ms`), which breaks "push never blocks". Replay return values are ignored (1031-1033), so a replay over about 64 frames can overflow the remote writer queue, and the audio is lost once the buffer has been taken (1013). | Do the switch off the audio thread; check `push_audio` during replay. |
| C4 | CONFIRMED | `crates/flow-router/src/router.rs:476`, 484-489; `crates/flow-server/src/ws.rs:459-460`, 570-576 | Cancel during commit does nothing. The router has already moved the session out of the mutex, and the server only drops its handle while the lease is inside `spawn_blocking`. A cancelled cloud commit runs up to 90 s and may then fall back to local ASR. The server worker stays busy, so the next session gets `busy` and falls back locally. | Keep a cancel handle (engine plus flag) reachable during commit and check it before each `switch_next`; on the server, hold `Arc<FlowCore>` and call `core.cancel()`. |
| C5 | CONFIRMED | `crates/flow-server/src/ws.rs:383`, 533-543 | The idle close runs only when no session is open, and `max_session_s` counts samples rather than wall-clock time. There are no server pings, so the 45 s dead rule (protocol.md:20) isn't enforced. A half-open phone after `start` holds the worker forever (`workers=1` → everyone else gets `busy`). | Apply the dead timeout regardless of session state; enforce a wall-clock session cap; send server pings. |
| C6 | PLAUSIBLE | `crates/flow-core/src/engine.rs:407-410`, 503, 743, 768; `router.rs:476` | A start that overlaps a commit (hands-free back-to-back) resets `context`, `dictionary` and `dropped` while the previous `finalize` is still reading them, so the earlier utterance is cleaned with the next app's context. | Snapshot the per-session state into `Ctrl::Commit`. |
| C7 | CONFIRMED (race) | `crates/flow-router/src/router.rs:398-403` | Two concurrent `start_session` calls: the second overwrites `session` without cancelling the first, orphaning a remote or cloud session and its lease. | `if let Some(old) = mem::replace(..) { old.engine.cancel() }`. |
| C8 | PLAUSIBLE | All `.lock().unwrap()` in `router.rs`, `engine.rs`, `apps/flow-windows/src/hotkeys/win.rs:238` | One panic while a lock is held poisons it, and the Router or core stays dead until restart (FFI turns every later call into an error). In the Windows LL hook callback, a poisoned lock panics inside an `extern "system"` callback, which aborts the process. | `unwrap_or_else(PoisonError::into_inner)`; reset the session state on recovery. |
| C9 | CONFIRMED | `crates/flow-router/src/router.rs:1010-1011`, 417-420 | Switching to a Local hop while later hops remain clones the buffer and keeps appending to it. Local never fails over, so up to about 38 MB is held for nothing. | When `hop.kind == Local`, take the buffer and clear `fallbacks`. |
| C10 | CONFIRMED | `crates/flow-router/src/router.rs:687-693` | `viable_after` treats cloud as usable whenever its circuit is closed, even without a key, consent or budget. With order `[server, cloud]`, the server circuit open and cloud unconfigured, the server is skipped and the dictation fails when a half-open server retry would have worked. | Include a read-only `cloud_gate().is_ok()` in `viable_after`. |
| C11 | PLAUSIBLE (high impact) | `apps/flow-windows/src/hotkeys/win.rs:250-258` | The physical Win/Alt key-up is swallowed and replaced by a masked injected release, without checking the `SendInput` result. Over an elevated window, UIPI drops the injected keys, so Win stays logically down (then L locks the PC, D shows the desktop). | If the foreground window has higher integrity or `SendInput` returns less than requested, let the physical key-up through. |
| C12 | CONFIRMED | `apps/flow-windows/src/dictation.rs:317-323`, `controller.rs:177-209` | Esc after `Effect::Paste` has been dispatched: the paste still happens, the session is dropped, so the transcript is missing from history and last-transcript. The 35 s timeout has the same race. | Check a cancel flag just before the chord; keep the session until `PasteFinished`. |
| C13 | PLAUSIBLE | `apps/flow-windows/src/input.rs:66-81`, `hotkeys/chord.rs:357-362` | Paste-last (Shift+Alt+Z) fires while Shift is still held, and only Alt/Win are released, so the app receives Ctrl+Shift+V (Markdown preview in VS Code, paste-with-formatting in Word). `release_before_paste` also releases a Ctrl the user is physically holding. | Masked-release Shift too, or wait for the chord's modifiers to come up; pass `lctrl_held`. |
| C14 | CONFIRMED | `apps/flow-windows/src/paste/win.rs:239`, 255-259 | `our_seq` is read before `CloseClipboard`, and the restore compares the sequence and then retries `OpenClipboard` for up to 250 ms. A user copy in that gap gets overwritten. | Read the sequence after close; re-check it after the restore's open. |
| C15 | CONFIRMED | `apps/flow-android/.../audio/MicRecorder.kt:57`, 75-81 | A read error `break`s silently: `recording=true` stays set, the recorder isn't released, and the mic foreground service stays up for up to 5 minutes. If the thread survives the 2.5 s stop timeout, its tail flush feeds old audio into the next session (or a freed handle, see C1). | Report the error to the controller; gate `onFrame` on a session generation. |
| C16 | PLAUSIBLE | `apps/flow-android/.../core/NativeFlowCore.kt:16`, `DictationController.kt:208-213` | `h()` throws `IllegalStateException`, but the controller catches only `CoreException`, so starting on an unloaded engine crashes `FlowService`. | Throw `CoreException`. |
| C17 | CONFIRMED (low) | `crates/flow-server/src/ws.rs:305` | The outbound channel is unbounded. A token holder that never reads but keeps sending pings or bad frames grows the server's memory without limit. Only `max_message_size` is set (`lib.rs:237`); the 16 MiB default frame size still applies. | Bounded channel that closes the connection when full; set `max_frame_size`. |
| C18 | CONFIRMED (low) | `crates/flow-core/src/engine.rs:557-565`; `router.rs:418`; `audio.rs:21` | Once `max_samples` is reached, `take(room)` frees only the iterated samples, so the ring fills up and every later push counts as dropped. A commit-time replay past the cap reports `truncated=false`. `resample_to_16k(_, 0)` divides by zero. | Drain the whole chunk; set `truncated` from the buffered count; guard `rate == 0`. |

## P3: smaller cloud and shell gaps

* `crates/flow-router/src/router.rs:144-149`, 305, 582: keyless self-hosted
  openai_compat servers can never be used, although the adapter and the
  Android hint support them. CONFIRMED.
* `router.rs:~950-968`, `cloud.rs:365`: the monthly budget counts only
  successful calls. Custom providers without a price add 0, and the budget is
  checked only at start. CONFIRMED.
* `apps/flow-android/.../data/CloudSettings.kt:277-281`: the consent banner
  always mentions dictionary words and never mentions on-screen words, so the
  text doesn't match the actual toggles. CONFIRMED.
* `apps/flow-android/.../insert/TextMatching.kt:9-21`, 53-59:
  * Success is "substring anywhere within ±500 chars". A failed `commitText`
    into a field that already contains "ok" or "thanks" is reported as
    inserted, so the transcript is lost.
  * Text that matches the `contentDescription` is treated as a placeholder,
    and `ACTION_SET_TEXT` then overwrites real user text.
  * Both PLAUSIBLE.
* Android Main-thread work: `nativeUnload` and `history.purge`
  (`SettingsActivity.kt:111-112`), and accessibility-tree walks of up to
  2 × 2 s (`FlowAccessibilityService.kt:202,280`). PLAUSIBLE.
* Windows: the controller blocks on `start_session` (3 s `LOAD_WAIT` plus the
  router connect, `controller.rs:383`) before it opens the mic, so first words
  are lost. A mic stream error (`audio/capture.rs:46`) is only logged, so a
  hands-free session keeps listening to nothing. CONFIRMED.
* flow-server:
  * A failed `start_session` (dead worker) is ignored and `Started` is sent
    anyway (`ws.rs:491`, 596). PLAUSIBLE.
  * `hello_ok.max_samples` reports the default rather than the server limit
    (`health.rs:446`).
  * The systemd `IPAddressAllow` omits 169.254/16 and non-tailnet fc00::/7,
    which `net.rs` treats as LAN.
* `crates/flow-ffi/src/jni_api.rs:181`: `get_array_length(..).unwrap_or(0)`
  can leave a Java exception pending. The JNI load path doesn't clear the last
  error on success, unlike `lib.rs:396`. PLAUSIBLE (low).

## P4: test gaps

* Cloud:
  * No redirect test (302/307 per adapter).
  * `no_key_leak.rs` covers only openai_compat and assumes the key travels in
    `authorization` (misses `xi-api-key`).
  * No test for stored key + changed `base_url` in `probe_cloud`.
  * No backslash or odd-URL host-parse cases.
* Router:
  * Nothing exercises `endpoint_reached` across a fallback replay.
  * No test that `set_context` is replayed after a switch; the fakes in
    `tests/cloud_policy.rs:87` and `tests/local_core.rs:80` ignore context.
  * No tests for: cancel during commit or fallback; concurrent start/start
    and start/commit; toggles changed mid-session (S5); stale partial after
    cancel→start (S11); replay past `max_samples`.
* Server:
  * No header, handshake or idle-with-session timeouts; no connection cap.
  * No clamp tests for `ring_capacity_samples`/`max_samples`.
  * No trace-level no-text check.
  * No test that a public or `0.0.0.0` bind is refused, or that cancel
    during commit frees the worker.
* Shells:
  * `apps/flow-windows/src/paste/win.rs` has no tests.
  * Nothing covers parallel paste-last, Esc after the paste was dispatched,
    or the start window vs the paste window.
  * `ManifestPrivacyTest` doesn't assert `allowBackup=false`,
    `dataExtractionRules`, or `CAP_CLOUD == (flavor != "local")`.
  * `tools/check-build-matrix.sh` runs `cargo tree` for the host target
    only. Add `--target` for the Windows and Android triples so
    target-specific dependencies are covered.

## P5: simplification

* Two hand-rolled URL/host classifiers (`crates/flow-router/src/cloud.rs`
  `split_url` + `net.rs`, and `apps/flow-android/.../CloudSettings.kt`), plus
  the Windows `dev_settings.rs:640-665` copy, have already diverged from the
  parser that makes the request (S2, S6). Use one `url`-crate–based check in
  flow-router, and have the shells call it over FFI/IPC instead of
  re-implementing it.
* Session-scoped state in `flow-core` lives in `Shared` (context, dictionary,
  partial, dropped, stop_partial) and is reset in `start_session`. Moving it
  into a per-session struct owned by the worker removes S11, C6 and part of
  C18 in one change.
* Router cancel/commit/switch logic would be simpler with an
  `Arc<ActiveSession>` (engine, buffer, cancel flag) that stays reachable
  during commit instead of `Option::take()` (fixes C4 and C7 together).

## Verification run

`cargo test --workspace --offline` (default features, separate target dir)
on 49b21c5: green (413 passed, 0 failed, 2 ignored across the first 60 test
binaries; output capped at 60 lines). Real-model and
all-features suites were not re-run for this review.
