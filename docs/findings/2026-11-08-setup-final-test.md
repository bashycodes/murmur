# 2026-11-08: setup's "Now try it" did nothing on a clean install (D75)

**Report.** Owner walkthrough of a clean Windows install (notetaker build
`7bd69c0`): the last setup step says "hold Right Alt and talk into the
box"; holding Right Alt and talking put no text in the box and the step
showed nothing. The app log at startup said the speech model was not found
(`…\models\ggml-base.bin … Nothing is downloaded automatically.`) and
`[hotkeys] keyboard hook stopped receiving events; re-installing`.

**Evidence (read-only, owner's PC, copies kept out of the repo).**

- `%APPDATA%\FlowClone\prefs.json`: `asr_engine` whisper, `model_path`
  null, `developer.allow_server` true, `server.host` empty,
  `route_order` [server, client]: the shipped engine defaults.
- No `models` folder under `%APPDATA%\FlowClone`: no speech model was
  downloaded or chosen.
- `history.sqlite`: 0 rows: no dictation ever finished, not even a failed
  one.
- Debug log off, so no per-dictation lines.

## Root cause

With no speech model on the PC and no server address, no dictation engine
can start. A press then fails at session start: the router returns
`LocalNotReady` (the server route is skipped for having no host), the
controller turns the start error into the pill's error state, and that is
all. The setup step only listened for a *finished* dictation
(`dev:route` from the router's route callback, `history:changed`), and a
start failure produces neither. So the step stayed silent by design, and
nothing checked before the user talked whether a press could work at all.

Hypotheses checked:

1. **No engine can start (no model, no server).** Confirmed by the prefs,
   the missing models folder and the empty history above, and by the code
   path (`RouterAdapter::start_error` → `Input::StartFailed` → pill only).
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

Open question: the saved engine settings are the defaults, although the
engine step saves the chosen engine (this PC first, or the server with
its address). Either setup was run again or skipped later, or the choice
did not persist. Another branch (`feat/setup-bad-config`) looks at what
setup saves.

## Fix (D75)

- `RouterAdapter::readiness()` and command `setup_readiness`: walks the
  dictation route order like a start, without a session or audio. A model
  that is ready or still loading counts; a configured server gets the same
  probe as Test connection; the cloud route only its key/consent check.
  When nothing can start it returns the reasons in plain words ("There's
  no speech model on this PC yet…", "There's no server address yet…",
  "Your server at … can't be reached…").
- The step calls it on opening and shows "Dictation can't work yet. …"
  when nothing can start.
- The pill's `dictation:status` is also sent to the settings window; the
  step shows listening, working, and the pill's error or notice.

## Tests

- `router_adapter::tests::readiness_*` (5): no model and no host → both
  reasons; server up → remote; server down with a model → local; server
  only and down → "can't be reached" with the address; every engine off
  → the no-engine message.
- `ui/tests/setup-try.test.mjs`: the step's words for readiness and for
  each pill state.

Not covered by an automated test: the Windows-only wiring (Tauri event to
the settings window, the WebView step). Retest on the PC.
