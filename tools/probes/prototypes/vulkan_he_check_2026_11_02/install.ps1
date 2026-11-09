# Back up prefs, close the app, install the 1.0.0 hybrid NSIS build silently, report.
$exe = "$env:LOCALAPPDATA\Murmur\murmur-windows.exe"
$prefs = "$env:APPDATA\Murmur\prefs.json"
$bak = "$prefs.bak-" + (Get-Date -Format 'yyyyMMdd-HHmmss') + "-pre-1.0.0"
Copy-Item $prefs $bak; "prefs backup: $bak"
$inst = "$env:USERPROFILE\Downloads\Murmur_1.0.0_x64-setup_hybrid.exe"
"sha256: " + (Get-FileHash $inst -Algorithm SHA256).Hash.ToLower()
Get-Process murmur-windows -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2
$p = Start-Process $inst -ArgumentList '/S' -PassThru -Wait
"installer exit: $($p.ExitCode)"
Start-Sleep 3
Get-Process murmur-windows -ErrorAction SilentlyContinue | % { "running after install: $($_.Id) (stopping)"; $_ | Stop-Process -Force }
"version: " + (Get-Item $exe).VersionInfo.ProductVersion
Get-ChildItem "$env:LOCALAPPDATA\Murmur" -File | ? Name -match 'dll$' | % { "dll: $($_.Name) $($_.Length)" }
