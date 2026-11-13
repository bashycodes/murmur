# Windows end-to-end tests (no human)

`tools/e2e-windows/run.sh` drives the real Murmur app on the Windows
laptop from the Linux server. It does a fresh install, presses real keys,
plays synthetic speech into a virtual microphone, reads the typed text back
and grades it. Each run writes its own results folder, then puts the
owner's data back.

## How it works

| Piece | What it does |
|---|---|
| SSH (ControlMaster) | Runs `tools/e2e-windows/ps/*.ps1` on the laptop with `powershell -File` (they are copied into `%USERPROFILE%\murmur-e2e` first). |
| `itask.ps1` | Runs something in the owner's **interactive** session through a one-shot scheduled task, which is deleted as soon as it starts. RunLevel is Limited: WebView2 ignores the debug-port variable when the app runs elevated. |
| `agent.ps1` | The hands in the interactive session: TCP JSON lines on 127.0.0.1:9224 through an SSH tunnel. It does SendInput keys (tap, hold N ms, double-tap with a gap), WAV playback into "CABLE Input", window list, focus and screenshots (PrintWindow of one window, never the screen), and our text pad (`pad.ps1`) read through UI Automation or WM_GETTEXT. For scenario h it also runs an installer and waits for it (`run_wait`), and drives the NSIS uninstaller's confirm page through UI Automation (`nsis_uninstall`: ticks or leaves the *Delete the application data* box, presses Uninstall, then Close). It quits on request or after 10 idle minutes. |
| `launch-app.ps1` | Starts the app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`. The harness talks CDP to its webviews (settings, overlay, notes) through a tunnel. |
| `appdata.ps1` | backup / wipe / seed / restore of the owner's data (see below). |
| `install.ps1` | Silent uninstall (`/S`), then silent install of the given installer. `-VersionOnly` only reads the installer's version. |
| `uninstall-data.ps1` | Scenario h: `check` reports whether `%APPDATA%\Murmur`, `%APPDATA%\FlowClone` and `%LOCALAPPDATA%\dev.murmur.windows` exist and whether Murmur is installed; `seed` writes dummy data there, and refuses unless the owner's data is in this run's backup. |
| `pad.ps1`, `pads.ps1`, `common.ps1` | Our text pad (a window titled `murmur-e2e-pad <name>`); the clean-up that stops leftover pads; the shared pad-title constant and matchers. |
| `lock.ps1` | Time since the laptop lock's last heartbeat. |
| `run-step.ps1`, `stepguard.ps1`, `lib/stepmark.mjs` | Every state-changing script runs as a *step* through `run-step.ps1`: it writes `murmur-e2e\step.json` (pid, script, start) and removes it in its own `finally`. See "Orphaned laptop steps". Read-only calls (status, lock age) run directly. |
| `lib/ssh-shield.sh` | Every ssh and scp runs under it, in its own session: a terminal Ctrl+C does not reach it. See "Interrupting a run". |
| VB-Cable | Virtual audio cable. Clips are played into "CABLE Input"; the app records from "CABLE Output". |
| `clips/clips.json` | Made-up sentences (English, Hebrew, mixed), voiced by espeak-ng on the server. No real speech. |

## The owner's data

`appdata.ps1` keeps a journal: `%USERPROFILE%\murmur-e2e\backups\<run id>\manifest.json`
plus a `backups\PENDING` marker. It covers these folders:

- `%APPDATA%\Murmur`
- `%APPDATA%\FlowClone`: the old folder, copied once into Murmur on first start (D83)
- `%LOCALAPPDATA%\dev.murmur.windows`: WebView2 data

**Backup** writes the journal and `PENDING` first, recording whether the
app was running. It then stops the app and **moves** each folder into the
backup. A move is a rename on the same volume, so a folder is always on one
side or the other. The journal is updated after every move: it is written
to `manifest.tmp` and swapped in with `File.Replace`, which keeps the
previous version as `manifest.json.bak`, so a full manifest exists at every
instant (restore falls back to the `.bak`). Each folder has one of these
states:

| State | Meaning |
|---|---|
| `absent` | The owner had no such folder, so a live one is test data. |
| `moving` | About to move. |
| `backed_up` | The owner's copy is in the backup, so a live one is test data. |
| `restored` | The owner's copy is live again. It is never touched. |

The rules:

- A live folder is deleted only when its label is `absent`, or when the owner's copy is still in the backup.
- `wipe` and `seed` refuse to run if any folder might be the owner's.
- **Restore** moves back only what is still in the backup and records each label it restores. A label the journal calls `backed_up` but that is no longer in the backup was moved back by an earlier, interrupted restore, and restore says so.
- A backup or restore cut off at any point can be finished by running restore again. Restoring twice is safe.
- `run.sh selftest` checks all of this on fake folders, never the owner's.
- While `PENDING` exists, a new backup is refused, so test data can never be saved as if it were the owner's.

Restore runs at the end of every run, including runs that error out; only
`--no-restore` skips it. Each clean-up step runs on its own, so one
failing does not skip the others. Restoring the folders keeps the server
key (DPAPI blob in prefs.json), the models and the history. If the app was
running before the run, it is started again normally, without the debug
port. Test data (debug log, history, notes) is copied into the results
only while the owner's data is in the backup.

Cloud keys in Credential Manager are not touched.

If a run is killed half way, restore by hand: `tools/e2e-windows/run.sh restore`.
`run.sh status` shows the journal.

The dictation scenarios (c–f) use a **seeded** profile. It is the owner's
prefs.json with these keys replaced (`SEED_PATCH` in `harness.mjs`):

| Key | Value | Why |
|---|---|---|
| `microphone` | `CABLE Output (VB-Audio Virtual Cable)` | The clips come in through the cable. |
| `onboarded` | `true` | No first-run setup. |
| `enable_sounds` | `false` | Quiet on the owner's laptop. |
| `mute_audio_while_dictating` | `false` | Leaves other apps' volume alone. |
| `launch_at_login` | `false` | Never registers the test install to start at login. |
| `keyboard_language` | `false` | Language by detection, not by keyboard layout (scenario f). |
| `languages` | `[]` | No pinned language (scenario f). |
| `developer.debug_log` | `true` | The other `developer` keys are kept. |

The owner's models are **copied** into the profile. A hard link would share
the file, so a test app rewriting a model in place would change the
owner's copy. Copying about 1.8 GB adds a few seconds. Scenario b copies
one small Whisper file the same way.

Prefs values are never printed. The scenarios read only `onboarded`,
`microphone` and `shortcuts`. Scenario d also reads three keys of the
owner's backed-up prefs: sounds, ducking and mic. This goes through
`appdata.ps1 -Action ownerprefs`, which allows only harmless keys.

## Several agents, one laptop

Runs queue: `run.sh` takes a `flock` on `~/.cache/murmur-e2e-laptop.lock`
(override with `MURMUR_E2E_LOCK`), and the harness also takes a lock on the
laptop. The laptop lock is a directory, `murmur-e2e\lock`, holding
`owner.txt` (host, pid and time). It catches runs from other machines.

- It is taken together with its owner: the harness builds `lock.tmp-<host>-<pid>\owner.txt`, then renames it to `lock`, which fails if `lock` exists.
- While the harness runs, a detached local loop touches `lock\heartbeat` every minute (only while `owner.txt` is still its own), so a run of any length keeps its lock.
- A lock is stale when its process is dead on this host, or when it has had no heartbeat for 10 minutes (counted from the newest of the dir, `owner.txt` and `heartbeat`). This covers lock dirs without `owner.txt` too.
- A stale lock is renamed aside, then deleted, so two runs cannot both break it. Before it is deleted, a step its owner left running on the laptop is waited for and stopped (see "Orphaned laptop steps").
- While waiting, the harness backs off from 2 s up to 20 s between checks and gives up after 2 h. An unreachable laptop is an error (exit 1), never taken for a stale lock.
- Both locks wait by default. `--no-wait` fails at once instead, with exit code 4.
- `status` and `clips` take no lock. `selftest` takes it, since it runs as a step.

## Setup (once)

1. Make `ssh <user>@<laptop>` work from the server without a password (key auth). Then name the laptop in one of these, highest priority first:
   - `--host user@laptop`
   - the `MURMUR_E2E_HOST` environment variable
   - `MURMUR_E2E_HOST=user@laptop` in `tools/e2e-windows/local.env`, which is gitignored

   There is no built-in default: without any of them, the harness stops with an error that says so.
2. Put the installer in the laptop's `Downloads`, or pass a local file with `--installer-local`.
3. Install VB-Cable: `run.sh vbcable-install` (or let the first run do it).
   - It downloads the official Pack46 zip and runs `VBCABLE_Setup_x64.exe -i -h`.
   - It installs silently with no UAC prompt over SSH; the driver is WHQL-signed.
   - The installer makes CABLE the default playback and recording device. The script saves the defaults first and puts them back.
4. Nothing else. espeak-ng is fetched into `~/.cache/murmur-e2e` (user space, no root).

## Run

```sh
tools/e2e-windows/run.sh --installer Murmur_1.3.0_x64-setup_hybrid.exe
tools/e2e-windows/run.sh --installer Murmur_1.3.0_x64-setup_hybrid.exe --scenarios c,f
tools/e2e-windows/run.sh --installer ... --scenarios d --tries 20 --keys RAlt --profiles owner --layout he --altgr
```

**Testing a branch's CI build.** The `windows-build` workflow uploads the
installers as the artifact `murmur-windows-hybrid-<label>`, where the label
is the version and short SHA. Download it and pass the file:

```sh
gh run list -R bashycodes/murmur -w windows-build -b feat/<slug> -L 1   # the run id
gh run download -R bashycodes/murmur <run-id> -p 'murmur-windows-hybrid-*' -D /tmp/murmur-ci
tools/e2e-windows/run.sh --installer-local /tmp/murmur-ci/murmur-windows-hybrid-*/Murmur_*_x64-setup_hybrid.exe
```

`--installer-local` copies the file to `murmur-e2e\installers\` on the
laptop, then installs it.

**The test build stays installed.** The run replaces the owner's Murmur
install with the installer under test and leaves it installed afterwards;
only the owner's data, audio devices and running app are put back. The
owner's installed version is recorded first (`owner_version` in
`results.json`). An installer older than it is refused, so a run never
silently downgrades the owner. `--allow-downgrade` overrides that. The
installer's version comes from its file name (`Murmur_1.3.0_...`), or else
from its file version info.

**Exit codes**, for gating:

| Code | Meaning |
|---|---|
| 0 | Every scenario passed, no clean-up step failed, and the restore of the owner's data was confirmed. |
| 1 | The run itself failed (see `results.error`), or the laptop was unreachable. |
| 2 | Bad arguments, or no laptop host set. |
| 3 | A scenario failed, errored or was aborted, a clean-up step failed, or the restore did not confirm. |
| 4 | The laptop is busy (`--no-wait`, or the wait timed out). |
| 5 | `--no-restore`: the test install's data is still live. `results.json` `pass` has the verdict. |
| 130 | Interrupted by SIGINT, SIGTERM or SIGHUP. The clean-up ran first. |

`results.json` has a top-level `"pass": true|false` (`lib/verdict.mjs`).
`results.restored` is set to `false` before the restore step and becomes
`true` only when restore confirms.

**Interrupting a run** (Ctrl+C, `kill`) is safe: the first signal runs the
same clean-up chain as the end of a run (stop pads, restore the owner's
data, relaunch the owner's app, put the audio defaults back), then exits
130. The ssh and scp calls run under `lib/ssh-shield.sh`, in their own
session, so the first Ctrl+C never kills a laptop step half way (an install
or restore finishes); the harness acts on the signal as soon as the current
step ends.

A **second Ctrl+C within 3 s** forces it: the shield stops the ssh in
flight, the harness stops the laptop step through its marker pid, then runs
the clean-up. Both say so on the terminal. A step stopped this way is a
clean-up error, so the run cannot pass; if it was the restore, run
`run.sh restore`. Signals after that are ignored.

**Orphaned laptop steps.** A step can outlive its ssh: a SIGKILL of the
harness, a forced Ctrl+C, or a spawnSync timeout (300 s for an install)
kills the local ssh, and Windows lets the PowerShell on the laptop run on.
So before every step, before `run.sh restore` and before breaking a stale
lock, the harness checks `murmur-e2e\step.json`. If its process is still
alive (same pid and start time), the harness waits for it (up to 90 s),
then stops its process tree (`taskkill /T /F`) and records a clean-up
error. A marker whose process is gone is just removed. After a timeout the
same check runs at once, with a 30 s wait.

A full run takes about 15 minutes, most of it scenario d. Results go to
`tools/e2e-windows/results/<stamp>-<installer>/`, which is gitignored. A
new folder is made per run.

| File | Contents |
|---|---|
| `summary.md` | Overall pass/fail, a table of scenarios, the double-tap table, and each scenario's evidence lines. |
| `results.json` | Everything, including per-try details. |
| `run.log`, `install.log` | Logs of the run and of the install. |
| `shots/` | CDP renders of webviews and PrintWindow shots of single windows. |
| `clips/` | The WAVs that were played. |
| `app-data/` | From the test profile: `dictations.jsonl` debug log, `history.sqlite`, notes. |

The laptop must be unlocked: the agent checks that the input desktop is
usable and aborts otherwise.

## Scenarios

| Id | Checks | Pass when |
|---|---|---|
| a | First launch after a fresh install (no data dirs at all) shows first-run setup. Records the DOM state, `onboarded`, whether prefs.json exists, visible windows, and whether the window is foreground or covered (info only). | A visible "Murmur" window with onboarding open. |
| g | Setup left half way (runs between a and b): saves the prefs the way a setup step does, so `prefs.json` exists with `onboarded` false, then restarts the app. | A visible "Murmur" window with onboarding open again and `onboarded` still false. |
| b | Walks onboarding through CDP with one shot per step. Steps: welcome → engine "this PC" → model (a copy of the smallest Whisper `.bin` from the owner's backed-up models) → cleanup → try (selects CABLE as the mic, holds Right Alt over a clip with the practice box focused) → Finish. | Every step is reached and `onboarded` is true. The practice-box dictation is reported as a sub-check. |
| c | Hold-to-talk: Right Alt held over two English clips, with our text pad focused; the text is read back from the pad. | Each transcript matches loosely: word-level similarity, normalized case and punctuation. |
| d | Double-tap over every combination of profile × key × tap length × gap, `--tries` each, round-robin, with ±`--jitter` ms on every tap and gap. See the next section. A try locks if the overlay pill reaches `active_locked` within 1.5 s; Esc then cancels. Every try is graded, slow ones too: its measured press-to-press time tells whether it was inside the app's 500 ms window. | Every combination on the bound push-to-talk key whose nominal tap + gap is at most 450 ms locks at least 90% of the time. |
| e | Settings → Scratchpad → New note: the notes window and its webview (URL, readyState, editor), then a dictation into the note and the notes folder on disk. | The editor loads and the clip lands in the note. |
| f | Hebrew, English and mixed clips into the pad with no language pinned. The language is read from the debug log (`language`, `language_source`) or, when the log has none (server route in 1.3.0), from the history entry, plus the share of Hebrew letters in the text. | Hebrew is detected as he with mostly Hebrew text, and English as en. The mixed clip is reported, not graded. |
| h | Runs last. The uninstaller's *Delete the application data* box. The box is only on the confirm page, which `/S` never shows, so there is no silent way to tick it: the agent starts the uninstaller in the interactive session and drives the page through UI Automation. Steps, each on dummy data seeded into `%APPDATA%\Murmur` and `%APPDATA%\FlowClone`: (1) uninstall with the box ticked; (2) reinstall, re-seed, uninstall with it unticked; (3) install over that data, then the same installer over the install with `/S`, then with `/P /UPDATE` (as the updater runs it). The test build stays installed. | (1) both folders and `%LOCALAPPDATA%\dev.murmur.windows` are gone; (2) and (3) both folders stay. The box starts unticked each time. |

### Scenario d dimensions

| Option | Values (default first) | Meaning |
|---|---|---|
| `--profiles` | `quiet,owner` | **quiet** is the seed: no sounds, no ducking, VB-Cable mic. **owner** uses the owner's sounds, ducking and mic settings, with the default speakers and mic muted for the duration and unmuted after. The app does the same work, but nobody hears it and nothing is recorded. |
| `--keys` | `RAlt,LAlt` | Only the key bound to push-to-talk (`shortcuts.ptt`) is graded. The other is reported as unbound. |
| `--tap-ms` | `80,120,160` | How long each tap is held. |
| `--gaps` | `120,200,300,400` | From release to the second press. |
| `--tries` | `4` | Tries per combination. |
| `--jitter` | `20` | Random ±ms on every tap and gap; `0` is exact. |
| `--altgr` | off | Also Right Alt as an AltGr layout sends it: a fake Left Ctrl (scan 0x21D), then Right Alt. |
| `--layout he` | off | Switches the pad to the Hebrew layout (which has AltGr) and back afterwards. |

`summary.md` gets one row per combination: locked/tries, aborted tries,
how many tries fell outside the 500 ms window, and the min/median/max
press-to-press time.

## Limits

- **Text-to-speech, not people.** espeak-ng is robotic, and its Hebrew is weaker than its English. Low scores on Hebrew may be the voice, so check the evidence lines (what was heard) before blaming the model. The laptop has no Hebrew Windows voice.
- **Foreground guard.** Keys are pressed only into our own window (a pad, or the app's settings or notes window). Processes started from a scheduled task are not always allowed to take the foreground; the agent's `focus` uses the usual workarounds.
  - If the target is not in the foreground, nothing is pressed and the try is recorded as **aborted**.
  - The agent checks the foreground again before the last release (`talk`, and each press of `double`). If the target lost it, the keys are released and Esc is sent at once to cancel the dictation. Esc comes after the release because Esc with Alt held is Alt+Esc, which switches windows.
  - A scenario whose tries were all aborted reports `aborted`, which fails the run.
- **Owner on the laptop.** The run presses real keys and opens text pads and app windows, so typing at the same time spoils the run.
  - The text target is our own pad (`ps/pad.ps1`: a window titled `murmur-e2e-pad <name>` with one text box), never Notepad: Windows 11 Notepad opens files as tabs in the owner's open Notepad window, so keys and pasted text could reach one of the owner's tabs. A pad saves nothing, is closed after each use, closes itself after 15 minutes, and the clean-up stops any left behind (`pads.ps1`).
  - Screenshots are of single windows only, never the screen.
  - No scheduled task stays behind; the run checks for leftover `MurmurE2E*` tasks.
- **CDP needs a non-elevated app.** If the owner ever runs Murmur elevated, the debug port does not open. The alternative, the HKCU WebView2 policy key, is not used.
- **The harness's own laptop footprint.**
  - `murmur-e2e\` holds the scripts, clips, installers copied with `--installer-local`, and the backups. A restored backup leaves an empty folder with its journal.
  - The audio defaults and mute state are saved at the start (`murmur-e2e\audio-defaults.json`) and put back at the end, which deletes the file. A run refuses to start while an earlier run's file is still there, because that file holds the owner's real defaults; `run.sh restore` puts them back.
- **Installer quirks.** The exe path must have no spaces, which holds for the default per-user install.
- **No cloud keys and no server changes.** The home server's container is only used, never changed.
