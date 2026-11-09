# Launch the installed app in the interactive session (one-shot scheduled task), stderr to a log,
# with prefs.hebrew_model set, then report: loaded whisper/vulkan modules, GPU memory, log tail.
# Env: HEB (hebrew_model, default ivrit_turbo); DEVICE=cpu -> MURMUR_WHISPER_DEVICE=cpu;
#      NOPLUGIN=1 -> run a temp copy of the install folder without murmur_whisper_vulkan.dll.
#      TAG names the log.
Get-Process murmur-windows -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 2
$heb = if ($env:HEB) { $env:HEB } else { 'ivrit_turbo' }
$py = "import json,os;p=os.path.join(os.environ['APPDATA'],'Murmur','prefs.json');j=json.load(open(p,encoding='utf-8'));j['hebrew_model']='$heb';json.dump(j,open(p+'.tmp','w',encoding='utf-8'),ensure_ascii=False,indent=2);os.replace(p+'.tmp',p);print('hebrew_model',j['hebrew_model'],'languages',j.get('languages'))"
& python -c $py
$dir = "$env:LOCALAPPDATA\Murmur"
if ($env:NOPLUGIN -eq '1') {
  $tmp = "$env:TEMP\lw-noplugin"
  if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
  Copy-Item $dir $tmp -Recurse
  Remove-Item "$tmp\murmur_whisper_vulkan.dll"
  Get-ChildItem $tmp -Filter 'uninstall*' | Remove-Item -Force
  $dir = $tmp
}
$exe = "$dir\murmur-windows.exe"
$log = "$env:TEMP\lw-check-$($env:TAG).log"
$cmd = "$env:TEMP\lw-launch-$($env:TAG).cmd"
$lines = @('@echo off')
if ($env:DEVICE -eq 'cpu') { $lines += 'set MURMUR_WHISPER_DEVICE=cpu' }
$lines += "`"$exe`" > `"$log`" 2>&1"
$lines | Set-Content -Encoding ASCII $cmd
$a = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument "/c `"$cmd`""
$pr = New-ScheduledTaskPrincipal -UserId (whoami) -LogonType Interactive
Register-ScheduledTask -TaskName MurmurLaunch -Action $a -Principal $pr -Force | Out-Null
Start-ScheduledTask MurmurLaunch; Start-Sleep 35
Unregister-ScheduledTask MurmurLaunch -Confirm:$false
$p = Get-Process murmur-windows -ErrorAction SilentlyContinue | Select -First 1; if (-not $p) { "app NOT running" } else {
  "pid: $($p.Id) session: $($p.SessionId) path: $($p.Path)"
  $p.Modules | ? { $_.ModuleName -match 'murmur_whisper|murmur_llm|vulkan|igvk' } | % { "module: $($_.ModuleName)" }
  try { (Get-Counter "\GPU Process Memory(pid_$($p.Id)*)\Dedicated Usage","\GPU Process Memory(pid_$($p.Id)*)\Shared Usage" -ErrorAction Stop).CounterSamples | ? CookedValue -gt 0 | % { "{0} = {1:N0} MB" -f $_.Path.Split('\')[-1], ($_.CookedValue/1MB) } } catch { "gpu counter: $_" }
}
if (Test-Path $log) { "--- log (filtered)"; Get-Content $log | ? { $_ -match '^\[|ggml_vulkan: \d|whisper_model_load: (model size|n_mels)|whisper_init|whisper_backend|Vulkan|GPU|error|fail' } | Select -Last 40 }
