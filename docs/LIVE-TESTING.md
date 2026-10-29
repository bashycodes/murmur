# Live testing: server, Windows, phone

One ordered pass over the whole system on real devices. Each step links to the
detailed guide instead of repeating it. Plan about 45 minutes. Note pass/fail
per checklist line and report as described in (e).

Builds under test: `integration` at the commit of the CI runs below.

## (a) Server on this Linux box (~10 min)

Full guide: [`docs/server-setup.md`](server-setup.md).

1. Prerequisites: the Parakeet model folder (`FLOW_PARAKEET_DIR`, or
   `models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`) and the sherpa-onnx
   libs (`SHERPA_ONNX_LIB_DIR`). This box, the Windows PC and the phone on one
   VPN (or one LAN).
2. Install as a user service (no sudo):
   ```sh
   tools/server/install.sh --user --dry-run   # review the steps
   tools/server/install.sh --user --start     # build, install, gen-token, config, start
   ```
   It binds `127.0.0.1:7313` plus this box's VPN IPv4 (100.64.0.0/10) on port **7313**.
   It writes the token once with `flow-server gen-token`; it never overwrites
   an existing one.
3. Collect the three values the apps need:
   ```sh
   ip -4 addr                               # host: the VPN 100.x.y.z (or LAN IP)
   cat ~/.config/flow-server/token          # token (copy over a trusted channel only)
   journalctl --user -u flow-server -f      # expect "ready workers=1 asr=parakeet … listening=…"
   ```
   Port = `7313`. Use TLS = off (the VPN already encrypts the traffic; on a plain LAN, consider TLS).
   `loginctl enable-linger $USER` (no sudo for yourself) keeps the server
   running after you log out; the installer asks for it when no systemd user
   manager is reachable (e.g. over ssh). If `systemctl --user` says "Failed to
   connect to bus", prefix it with `XDG_RUNTIME_DIR=/run/user/$(id -u)`.

## (b) Windows (~15 min)

Full guide: [`apps/flow-windows/README.md` "Live test on Windows"](../apps/flow-windows/README.md#live-test-on-windows-ci-installers).

1. Download the installer from the green integration run:
   <https://github.com/bashycodes/murmur/actions/runs/39839507123> (commit 30972dd). Artifact **`flow-clone-windows-hybrid`**
   (`gh run download 39839507123 -n flow-clone-windows-hybrid`). Unzip it.
2. Run `Murmur_<version>_x64-setup_hybrid.exe` (e.g. `Murmur_0.8.0_x64-setup_hybrid.exe`) (per user, no admin) or the
   `.msi`. SmartScreen: More info → Run anyway (the installer is unsigned).
   Install only one flavour at a time.
3. Optional local model, for the fallback: copy `ggml-base.bin` (or the
   Parakeet folder) to `%APPDATA%\FlowClone\models\`.
4. Settings → Developer: Host = the `100.x.y.z` from (a) (no scheme, no
   port), Port `7313`, Token = Paste, TLS off, Allow server + Allow client on,
   Save. **Test connection** must say `OK · median N ms · flow-server … ·
   parakeet`; **Test session** must say `EMPTY (no_speech) via server`.
5. Run the README's "10-minute smoke checklist".

## (c) Phone (~15 min)

Full guide: [`apps/flow-android/INSTALL.md` "Live test on phone"](../apps/flow-android/INSTALL.md#live-test-on-phone-vpn-10-minutes).

1. APK: **hybrid**. Either `flow-clone-hybrid-debug.apk` from the artifact
   **`flow-clone-android-debug-apks`** of <https://github.com/bashycodes/murmur/actions/runs/39837428823> (commit b9942f1; predates the 2026-10-23 model-import and a11y-health fixes from `fix/android-live`), or
   `apps/flow-android/dist/flow-clone-hybrid-debug.apk` built on this box
   (rebuild with `tools/dist.sh` from current `integration` to get those fixes; the SHA-256 values in INSTALL.md are for the earlier build). Check it with `sha256sum` before installing.
2. **Uninstall first** if Murmur is already on the phone and was built
   elsewhere: the CI APK and the local one are signed with different debug
   keys, and Android refuses one as an update of the other. Uninstalling
   deletes settings, dictionary and history.
3. Install from Files ("Install unknown apps" for Files). Open Murmur and
   grant in this order: *Microphone* → *Display over other apps* →
   *Accessibility service* (greyed out: App info → ⋮ → *Allow restricted
   settings*) → *Notifications* → *Battery: allow background*. The main
   screen must show `Native engine: flow-ffi <version>+<commit>` (e.g. `flow-ffi 0.2.0+1a2b3c4`; D26).
4. *Developer settings*: host = `100.x.y.z`, port `7313`, paste the token,
   *Use TLS* off, *Allow server* + *Allow client* on, *Save*, *Test
   connection* → `OK · median N ms … · parakeet`. For the phone fallback,
   import a model (INSTALL.md §5) or tick *Settings → Mock engine*.
5. Run INSTALL.md's "Smoke checklist (10 minutes)".

## (d) Optional: cloud keys (costs cents)

Only if you want to test third-party STT. Design and presets:
[`docs/cloud-stt.md`](cloud-stt.md).

* Windows: the cloud lines of the smoke checklist, and README "Cloud
  providers" for the full list.
* Phone: INSTALL.md §5c and the "Cloud STT (D18)" checklist.
* Self-hosted (speaches / LocalAI on the LAN or a VPN): pick preset
  **Custom**. It needs no API key on a private or VPN host. Named presets
  (Groq, OpenAI, …) always need a key.
* Host keys only: `FLOW_CLOUD_LIVE_<GROQ|OPENAI|…>_KEY=… cargo test -p
  flow-cloud --test live_smoke -- --nocapture`.

## (e) What to report for each failure

For every failed line: device (PC model / phone model + Android version),
the checklist line, what you saw, and:

| Where | Send |
|---|---|
| Server | `journalctl --user -u flow-server --since "10 min ago"` (it logs no text and no token) |
| Windows | the **Active engine** row on the Developer tab (route, fallback reason, rtt); `%APPDATA%\FlowClone\prefs.json` (no secrets in it); console output of `"C:\Program Files\Murmur\Murmur.exe"` started from a terminal (`[audio]`, `[hotkeys]`, `[cloud]` lines) |
| Phone | the **Active engine (last dictation)** row on the Developer page; the Test connection line; logcat for tags `FlowService`, `FlowEngine`, `FlowA11y`, `FlowMic` (over USB with the platform tools) |

Never paste the server token or a cloud key into a report.
