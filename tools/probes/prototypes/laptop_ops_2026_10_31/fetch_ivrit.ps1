# ivrit-ai whisper-large-v3-turbo f16 (pinned) -> verify -> whisper-quantize q5_0 into wispr-bench\models.
$m = "$env:USERPROFILE\wispr-bench\models"; New-Item -ItemType Directory -Force $m | Out-Null
$f16 = "$m\ivrit-turbo-f16.bin"; $q = "$m\ivrit-turbo-q5_0.bin"
if (-not (Test-Path $f16)) {
  & curl.exe -sSfL --retry 3 -o "$f16.part" "https://huggingface.co/ivrit-ai/whisper-large-v3-turbo-ggml/resolve/2130c78e4a9cb4914cc4df91a1c3031407789705/ggml-model.bin"
  Move-Item "$f16.part" $f16
}
$h = (Get-FileHash $f16 -Algorithm SHA256).Hash.ToLower(); "f16 $((Get-Item $f16).Length) $h"
if ($h -ne 'c8090411113357097bfafc2b8e228ec1639fa7f5fe4ecb5d054ac0ccef8641b1') { "F16 HASH MISMATCH"; exit 1 }
& "$env:USERPROFILE\wispr-bench\bin\whisper-cpu\Release\whisper-quantize.exe" $f16 $q q5_0 2>&1 | Select -Last 3
"q5_0 $((Get-Item $q).Length) $((Get-FileHash $q -Algorithm SHA256).Hash.ToLower())"
