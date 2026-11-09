# Laptop state before the check: installed version, plugin DLLs, models, bench folder, app process.
$exe = "$env:LOCALAPPDATA\Murmur\murmur-windows.exe"
"ver: " + (Get-Item $exe).VersionInfo.ProductVersion
Get-ChildItem "$env:LOCALAPPDATA\Murmur" | ? Name -match 'dll$' | % { "dll: $($_.Name) $($_.Length)" }
Get-ChildItem "$env:APPDATA\Murmur\models" | % { "m: $($_.Name) $($_.Length)" }
Get-ChildItem "$env:USERPROFILE\murmur-bench" | % { "b: $($_.Name)" }
Get-ChildItem "$env:USERPROFILE\murmur-bench\models" -ErrorAction SilentlyContinue | % { "bm: $($_.Name) $($_.Length)" }
"audio-he files: " + (Get-ChildItem "$env:USERPROFILE\murmur-bench\audio-he" -ErrorAction SilentlyContinue).Count
Get-Process murmur-windows -ErrorAction SilentlyContinue | % { "running pid $($_.Id) sess $($_.SessionId)" }
python --version
Select-String -Path "$env:APPDATA\Murmur\prefs.json" -Pattern 'hebrew_model' | % { "pref: " + $_.Line.Trim() }
