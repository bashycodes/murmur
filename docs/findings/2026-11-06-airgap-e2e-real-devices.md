# Air-gapped end-to-end check on real devices (2026-11-06)

Ticket "1.0.0 · 8". This is the manual checklist from
[`2026-11-01-airgap-e2e.md`](2026-11-01-airgap-e2e.md) run on the owner's
Windows PC and phone. Owner guide: [`docs/air-gapped.md`](../air-gapped.md).

## Setup

* flow-server 1.0.0+df49069 on the home machine.
* Windows app 1.1.2+794960b on the Windows PC.
* Android hybrid 1.1.2+0d47e56 on the phone (SM-S936B).
* Both clients reach the server over Tailscale.

## How "internet blocked" was done, and its limits

* **Windows:** an outbound Windows Firewall rule blocked `flow-windows.exe`
  from every address except the server's. Only the app was fenced, not the
  whole PC.
* **Phone: not isolated.** It stayed online, because Tailscale needs the
  internet to reach the server. The phone steps therefore prove that the
  app works through the server. They do **not** prove that it works
  without internet. A true air gap for the phone needs a LAN-only server
  address (no VPN), which was not tried.

## Results

| Step | Result |
|---|---|
| Server `library list` | 4 models: `parakeet-tdt-0.6b-v2-int8` and `zipformer-he` with a `catalogue_id`; `parakeet-unified-en` and `whisper-tiny-q5_1` without one. **No AI cleanup model** in the library |
| Windows (admin key): Your server → Test connection | PASS, "Connected. 8 ms" |
| Windows: Users & keys | PASS: shown; created key `e2e-phone`, secret shown once |
| Windows: Speech to text → From your server → Download Whisper tiny q5_1 (32 MB), through the firewall rule | PASS: "Checking the fingerprint…" then "Downloaded and in use". Server counted 1 download (32.2 MB) for the admin key |
| Windows: AI cleanup model from the server | **NOT RUN**: no cleanup model in the library |
| Phone: paste the `e2e-phone` key → Test connection | PASS, "OK · median 15 ms"; Users & keys not shown (non-admin) |
| Phone: Parakeet screen → From your server → Show models → Download Parakeet TDT 0.6B v2 (661 MB) | PASS: progress shown, fingerprint checked, "In use" |
| Phone: cut Wi-Fi mid-download → Resume | **NOT RUN**: the download finished before it could be cut. Resume is covered by `flow-remote/tests/library.rs` |
| Both: one English dictation each, with "Your server" first in the engine order | PASS: the server log shows a result for each key (~16.6 s of audio each); the phone history labels the entry "Your server" |
| Windows Users & keys after refresh | PASS: the `e2e-phone` row shows client `flow-ffi 1.1.2`, 1 dictation, 0.3 audio min, 4 downloads |
| Revoke `e2e-phone` from Windows Users & keys | PASS: confirm dialog shown; server log "admin: key revoked" |
| Phone Test connection after the revoke | PASS: "auth_failed: token rejected" |
| Next phone dictation | PASS: server log "rejected: bad or missing token"; the phone fell back to its own model, and history labels the entry "This phone" |
| Windows with the server stopped: Import from file… | **NOT RUN** |
| Local APK | **NOT RUN** (optional) |

The `e2e-phone` row counts 4 downloads although the phone installed one
model. This run did not check why.

## Not covered

* The phone was not offline (see above).
* AI cleanup served from the library, because the library has no cleanup
  model.
* Resume after a cut on a real device.
* Import from file with the server stopped.
* The local APK.
* Hebrew dictation through the server, even though `zipformer-he` is listed.

## Test method: driving the Windows app remotely

The Windows app's WebView2 was started with
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`.
That port was forwarded over an SSH tunnel, so a DevTools client on the
dev box could read and click the Settings UI. The same method can be
reused for other Windows UI checks.
