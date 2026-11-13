# Download Llama 3.2 3B Instruct Q4_K_M at the revision and SHA-256 the
# Windows app's catalogue pins (apps/murmur-windows/src/model_download.rs)
# into the bench folder. Run through itask.ps1 so SSH ending does not kill it.
$ErrorActionPreference = 'Stop'
$d = Join-Path $env:USERPROFILE 'murmur-bench'
$f = 'Llama-3.2-3B-Instruct-Q4_K_M.gguf'
$url = "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/$f"
$log = Join-Path $d 'logs\dl-llama.log'
"start $(Get-Date -Format s)" | Set-Content $log
& curl.exe -fsSL --retry 5 -o (Join-Path $d "models\$f.part") $url 2>&1 | Add-Content $log
Move-Item -Force (Join-Path $d "models\$f.part") (Join-Path $d "models\$f")
$h = (Get-FileHash (Join-Path $d "models\$f") -Algorithm SHA256).Hash.ToLower()
"sha256 $h" | Add-Content $log
if ($h -ne '6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff') { 'MISMATCH' | Add-Content $log } else { 'ok' | Add-Content $log }
