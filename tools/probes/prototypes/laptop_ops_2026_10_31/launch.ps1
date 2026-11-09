# Apply prefs (app closed) and launch the app in the user's interactive session via a
# one-shot scheduled task. LOGFILE env: when set, stdout+stderr go to that file.
Get-Process murmur-windows -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 1
if ($env:SETPREFS -ne '0') { & python "$env:USERPROFILE\murmur-bench\set_prefs.py" }
$exe = "$env:LOCALAPPDATA\Murmur\murmur-windows.exe"
$log = "$env:LOCALAPPDATA\Murmur\verify-0.12.0.log"
if ($env:REDIRECT -eq '1') { $a = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument "/c `"`"$exe`" > `"$log`" 2>&1`"" }
else { $a = New-ScheduledTaskAction -Execute $exe }
$pr = New-ScheduledTaskPrincipal -UserId (whoami) -LogonType Interactive
Register-ScheduledTask -TaskName MurmurLaunch -Action $a -Principal $pr -Force | Out-Null
Start-ScheduledTask MurmurLaunch; Start-Sleep 20
Unregister-ScheduledTask MurmurLaunch -Confirm:$false
$p = Get-Process murmur-windows -ErrorAction SilentlyContinue | Select -First 1; if (-not $p) { "app NOT running"; return }
"pid: $($p.Id) session: $($p.SessionId)"
$p.Modules | ? { $_.ModuleName -match 'murmur_llm|vulkan|ivulkan|igvk|sherpa|onnx' } | % { "module: $($_.ModuleName)" }
"working set MB: " + [int]($p.WorkingSet64/1MB)
try { (Get-Counter "\GPU Process Memory(pid_$($p.Id)*)\*" -ErrorAction Stop).CounterSamples | ? CookedValue -gt 0 | % { "{0} = {1:N0} MB" -f $_.Path.Split('\')[-1], ($_.CookedValue/1MB) } } catch { "gpu counter: $_" }
if (Test-Path $log) { "--- log"; Get-Content $log -Tail 80 }
