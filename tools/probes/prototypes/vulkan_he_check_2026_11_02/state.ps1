# Laptop state before the check: installed version, plugin DLLs, models, bench folder, app process.
$exe = "$env:LOCALAPPDATA\Murmur\flow-windows.exe"
"ver: " + (Get-Item $exe).VersionInfo.ProductVersion
Get-ChildItem "$env:LOCALAPPDATA\Murmur" | ? Name -match 'dll$' | % { "dll: $($_.Name) $($_.Length)" }
Get-ChildItem "$env:APPDATA\FlowClone\models" | % { "m: $($_.Name) $($_.Length)" }
Get-ChildItem "$env:USERPROFILE\flow-bench" | % { "b: $($_.Name)" }
Get-ChildItem "$env:USERPROFILE\flow-bench\models" -ErrorAction SilentlyContinue | % { "bm: $($_.Name) $($_.Length)" }
"audio-he files: " + (Get-ChildItem "$env:USERPROFILE\flow-bench\audio-he" -ErrorAction SilentlyContinue).Count
Get-Process flow-windows -ErrorAction SilentlyContinue | % { "running pid $($_.Id) sess $($_.SessionId)" }
python --version
Select-String -Path "$env:APPDATA\FlowClone\prefs.json" -Pattern 'hebrew_model' | % { "pref: " + $_.Line.Trim() }
