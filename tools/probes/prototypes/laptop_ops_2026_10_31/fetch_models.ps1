# Fetch the 0.12.0 catalogue files (pinned URL + size + SHA-256 from
# apps/murmur-windows/src/model_download.rs) into %APPDATA%\Murmur\models,
# at the paths the in-app downloader uses. .part then rename after the hash.
$ErrorActionPreference = 'Stop'
$m = "$env:APPDATA\Murmur\models"
$hf = 'https://huggingface.co'
$zb = "$hf/ivrit-ai/zipformer2-66m-streaming-he/resolve/ee769a2a13b887f91572eec349458bfb0fe79e7a/sherpa-onnx"
$files = @(
  @{d="Llama-3.2-3B-Instruct-Q4_K_M.gguf"; u="$hf/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/Llama-3.2-3B-Instruct-Q4_K_M.gguf"; s=2019377696; h="6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff"},
  @{d="ggml-tiny-q5_1.bin"; u="$hf/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny-q5_1.bin"; s=32152673; h="818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7"},
  @{d="zipformer2-66m-streaming-he-int8\encoder.int8.onnx"; u="$zb/encoder.int8.onnx"; s=70478964; h="6c6febcb74a09435315a1d872a6539f6e7fc2c333533f14c74edc0e85c49ffcd"},
  @{d="zipformer2-66m-streaming-he-int8\decoder.int8.onnx"; u="$zb/decoder.int8.onnx"; s=1049105; h="feb03e7c8aa761593a9284a6fa1e91259be3702de75f630062256c1a59c5a906"},
  @{d="zipformer2-66m-streaming-he-int8\joiner.int8.onnx"; u="$zb/joiner.int8.onnx"; s=771805; h="1d2c6d8e6dabc5eb52de52e95718e9bdfbffbfe7ead3808fb5ffd74e83bd9cae"},
  @{d="zipformer2-66m-streaming-he-int8\tokens.txt"; u="$zb/tokens.txt"; s=17086; h="e4ebb5e72a6aeef47437a55fa2308ebbedb7c51c557de7a89bf965c23ea9af4a"}
)
foreach ($f in $files) {
  $t = Join-Path $m $f.d
  New-Item -ItemType Directory -Force (Split-Path $t) | Out-Null
  if ((Test-Path $t) -and (Get-Item $t).Length -eq $f.s -and (Get-FileHash $t -Algorithm SHA256).Hash.ToLower() -eq $f.h) { "OK (present) $($f.d)"; continue }
  $part = "$t.part"
  & curl.exe -sSfL --retry 3 -C - -o $part $f.u
  if ($LASTEXITCODE -ne 0) { "FAIL curl $LASTEXITCODE $($f.d)"; continue }
  $len = (Get-Item $part).Length
  $hash = (Get-FileHash $part -Algorithm SHA256).Hash.ToLower()
  if ($len -eq $f.s -and $hash -eq $f.h) { Move-Item -Force $part $t; "OK $($f.d) $len $hash" } else { "BAD $($f.d) len=$len hash=$hash" }
}
