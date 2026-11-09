# Close the app, install the 0.12.0 hybrid NSIS build silently, report.
$exe = "$env:LOCALAPPDATA\Murmur\murmur-windows.exe"
Get-Process murmur-windows -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2
$p = Start-Process "$env:USERPROFILE\Downloads\Murmur_0.12.0_x64-setup_hybrid.exe" -ArgumentList '/S' -PassThru -Wait
"installer exit: $($p.ExitCode)"
Start-Sleep 3
Get-Process murmur-windows -ErrorAction SilentlyContinue | % { "running after install: $($_.Id) (stopping)"; $_ | Stop-Process -Force }
"version: " + (Get-Item $exe).VersionInfo.ProductVersion
Get-ChildItem "$env:LOCALAPPDATA\Murmur" -File | Select Name,Length | ft -auto | Out-String -Width 200
