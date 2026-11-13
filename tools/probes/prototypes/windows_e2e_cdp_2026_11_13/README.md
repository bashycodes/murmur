# Windows e2e harness probes (2026-11-13)

Read-only probes run on the laptop while building `tools/e2e-windows/`
(promoted tool). Copy one to the laptop's `murmur-e2e` folder and run it
with `powershell -File`.

- `recon-voices.ps1`: installed SAPI / OneCore voices. Only English ones,
  so the Hebrew clips come from espeak-ng instead.
- `recon-webview.ps1`: the WebView2 browser command line of the app.
- `recon-procenv.ps1`: `WEBVIEW2_*` variables in the app's process.
- `recon-pipe.ps1`: whether WebView2 connects to
  `WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER`.

Finding: `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port`
was in the app's environment but WebView2 (runtime 157) ignored it while
the app ran **elevated** (a scheduled task with `RunLevel Highest`). With
`RunLevel Limited`, as when the owner starts it, the port opens (tested
with the variable set through `ProcessStartInfo`; `Start-Process` was only
tried elevated).
`WEBVIEW2_USER_DATA_FOLDER` was honoured in both cases.
