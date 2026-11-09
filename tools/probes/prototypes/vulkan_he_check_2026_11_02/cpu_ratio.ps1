# Is the plugin's CPU path slow for every model or only ivrit q5_0? ggml-base (f16) on the 10.4 s
# clip: whisper-cli b5454 (CPU, OpenMP) vs the installed plugin with MURMUR_WHISPER_DEVICE=cpu.
$b = "$env:USERPROFILE\murmur-bench"
$m = "$env:APPDATA\Murmur\models\ggml-base.bin"
$wav = "$b\audio-he\4872784854716589132.wav"
foreach ($fa in @($false, $true)) {
  $args = @('-m', $m, '-f', $wav, '-t', '4', '-l', 'he', '-nt', '-bs', '1', '-bo', '1')
  if ($fa) { $args += '-fa' }
  $err = & "$b\bin\whisper-cpu\Release\whisper-cli.exe" @args 2>&1 | % { "$_" }
  $tot = ($err | Select-String 'total time =\s+([\d.]+)').Matches.Groups[1].Value
  $load = ($err | Select-String 'load time =\s+([\d.]+)').Matches.Groups[1].Value
  $enc = ($err | Select-String 'encode time =\s+([\d.]+)').Matches.Groups[1].Value
  "whisper-cli base fa=$fa compute_ms=$([double]$tot - [double]$load) encode=$enc"
}
$env:MURMUR_WHISPER_DEVICE = 'cpu'
Set-Location $b
& python plugin_bench.py --dll "$env:LOCALAPPDATA\Murmur\murmur_whisper_vulkan.dll" --model $m --audio audio-he --manifest he_manifest.json --ids 4872784854716589132 --warmup 0 --out results\vk-base-cpu1-20261102.jsonl 2>&1 | ? { "$_" -match '"kind": "(clip|info)"' } | % { "plugin: $_" }
