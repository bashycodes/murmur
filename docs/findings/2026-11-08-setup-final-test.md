# 2026-11-08: setup's "Now try it" did nothing (D75; walkthrough cause D76)

**Report.** Owner walkthrough on Windows: the last setup step says "hold
Right Alt and talk into the box"; holding Right Alt and talking put no
text in the box and the step showed nothing. The app log at startup said
the speech model was not found (`…\models\ggml-base.bin … Nothing is
downloaded automatically.`) and `[hotkeys] keyboard hook stopped receiving
events; re-installing`.

**What was installed.** Not a clean profile: the owner installed the
v1.1.2 GitHub release over a profile left by earlier builds. In that state
every setup save failed with `bad_config: invalid custom cleanup model id
""` (D76, fixed on its own branch), so the engine step's choice (and its
server address) was never stored.

**Evidence (read-only, owner's PC, copies kept out of the repo).**

- `%APPDATA%\Murmur\prefs.json`: `asr_engine` whisper, `model_path`
  null, `developer.allow_server` true, `server.host` empty,
  `route_order` [server, client]: the shipped engine defaults, consistent
  with every setup save being refused.
- No `models` folder under `%APPDATA%\Murmur`: no speech model.
- `history.sqlite`: 0 rows: no dictation ever finished, not even a failed
  one.
- Debug log off, so no per-dictation lines.

## Root cause

**The walkthrough failure is D76.** The refused saves left no engine set
up: no server address and no speech model. Every press then failed at
session start: the router skips the server (no host) and the local route
returns `LocalNotReady`; the controller turns that into the pill's error
state, and that is all.

**Why the step was silent (D75).** The step only listened for a
*finished* dictation (`dev:route` from the router's route callback,
`history:changed`), and a start failure produces neither. Nothing checked
before the user talked whether a press could work at all. The same silence
happens on a truly clean install where nothing is set up yet (the user
skipped the model and gave no server address), which is what D75 fixes.

Hypotheses checked:

1. **No engine can start (no model, no server).** Confirmed by the prefs,
   the missing models folder and the empty history above, and by the code
   path (`RouterAdapter::start_error` → `Input::StartFailed` → pill only).
   Why nothing was set up: D76.
2. **Engine choice applied only after a restart.** Ruled out: the engine
   step saves through `dev_set`, which applies to the next session without
   a reload (`settings_apply_to_the_next_session_without_reload`).
3. **Paste into the app's own window fails.** Not the cause here (nothing
   was ever transcribed). Own windows are pasted into like any app
   (`Controller::own_exe`, the Scratchpad path).
4. **Keyboard hook not receiving while the setup window has focus.** The
   hook is global (`WH_KEYBOARD_LL`) and does not filter our own windows;
   the watchdog line shows it noticed a dead hook and re-installed it. Not
   reproducible from logs. The fix makes this visible: a press that reaches
   the app now shows "Listening…" in the step, so a press that shows
   nothing at all means the shortcut never arrived.

## Fix (D75)

- `Router::route_checks()` (murmur-router, read-only, no network): the
  dictation routes a start would walk, in order, each with the reason a
  start would skip it without trying (no server host, an open or sticky
  circuit with a viable fallback, missing cloud consent/key/budget, no
  local engine). `RouterAdapter::local_first` uses it too, so the walk
  lives in one place next to `start_routes`.
- `RouterAdapter::readiness()` → `Readiness::Ready { engine }` or
  `NotReady { message }`, command `setup_readiness`. A model ready or still
  loading counts; a server a start would try is probed with
  `probe_server`, which, like a failed start, updates the server's circuit
  breaker and "Server info" health. No session, no audio. The message is
  the reason of the first route in the order, the engine the user chose:
  with "Your server" chosen and down, it names the server only.
- The step calls it on opening and shows "Dictation can't work yet. …";
  while that line is up it checks again every 5 s and when a model
  finishes downloading or loading, and clears the line once ready.
- The pill's `dictation:status` is also sent to the settings window; the
  step shows listening, working, and the pill's error or notice.

## Tests

- `router_adapter::tests::readiness_*` (6): server chosen, no host, no
  model → the server reason only; this PC chosen, no model → the model
  reason only; server up → remote; server down with a model → local, the
  circuit opens and the next check skips the server; server only and down
  → "can't be reached" with the address; every engine off → the no-engine
  message.
- `ui/tests/setup-try.test.mjs`: the step's words for readiness and for
  each pill state.

Not covered by an automated test: the Windows-only wiring (Tauri event to
the settings window, the WebView step, the 5 s re-check). Retest on the PC.
