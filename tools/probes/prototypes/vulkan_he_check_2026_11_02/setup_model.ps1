# Copy the 2026-10-31 bench's ivrit turbo q5_0 (byte-identical to the catalogue file, D42)
# into the app's models folder under the catalogue name; no download. Check size + SHA-256.
$src = "$env:USERPROFILE\wispr-bench\models\ivrit-turbo-q5_0.bin"
$dst = "$env:APPDATA\FlowClone\models\ggml-ivrit-large-v3-turbo-q5_0.bin"
if (-not (Test-Path $dst)) { Copy-Item $src $dst }
$f = Get-Item $dst
"size: $($f.Length) (want 574041195)"
"sha256: " + (Get-FileHash $dst -Algorithm SHA256).Hash.ToLower()
