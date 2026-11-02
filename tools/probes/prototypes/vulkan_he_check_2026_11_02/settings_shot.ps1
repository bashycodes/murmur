# Run settings_ui.ps1 (copied to the bench folder) in the interactive session via a one-shot
# scheduled task; print its text dump. The screenshot stays on the laptop (%TEMP%), never committed.
# Env: TAG names the output; NOPLUGIN=1 uses the temp no-plugin copy (app_check.ps1).
$dir = if ($env:NOPLUGIN -eq '1') { "$env:TEMP\lw-noplugin" } else { "$env:LOCALAPPDATA\Murmur" }
$out = "$env:TEMP\lw-settings-$($env:TAG).png"
Remove-Item "$out.txt" -ErrorAction SilentlyContinue
$ps = "$env:USERPROFILE\wispr-bench\settings_ui.ps1"
$a = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$ps`" -Exe `"$dir\flow-windows.exe`" -Out `"$out`""
$pr = New-ScheduledTaskPrincipal -UserId (whoami) -LogonType Interactive
Register-ScheduledTask -TaskName FlowCloneSettingsShot -Action $a -Principal $pr -Force | Out-Null
Start-ScheduledTask FlowCloneSettingsShot; Start-Sleep 25
Unregister-ScheduledTask FlowCloneSettingsShot -Confirm:$false
if (Test-Path "$out.txt") { Get-Content "$out.txt" -Encoding UTF8 } else { "no output" }
