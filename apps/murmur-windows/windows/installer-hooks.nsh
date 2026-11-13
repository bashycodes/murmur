; NSIS installer hooks (tauri.conf.json bundle.windows.nsis.installerHooks).
;
; Tauri's uninstaller box "Delete the application data" only removes
; %APPDATA%\<identifier> and %LOCALAPPDATA%\<identifier>
; (dev.murmur.windows: the WebView2 profile). The app keeps everything
; else in %APPDATA%\Murmur (prefs.json, history, dictionary, notes,
; meetings, insights, sounds, cache, debug log, models), and D83 copied
; the pre-rename %APPDATA%\FlowClone there. Without this hook an uninstall
; with the box ticked kept prefs.json (onboarded: true), so a reinstall
; skipped setup.
;
; Same condition as the template's own removal (tauri-bundler 2.10.4
; installer.nsi, Section Uninstall): the box ticked and not an update
; ($UpdateMode is 1 when the uninstaller runs with /UPDATE, as the
; installer does when it replaces an existing install). Unticked, or
; during an update, nothing is deleted. Folders the user picked outside
; %APPDATA%\Murmur (a custom notes or model folder) are never touched.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\Murmur"
    RmDir /r "$APPDATA\FlowClone"
  ${EndIf}
!macroend
