# A/B: whisper-cli (b5454 + llama.cpp b12005 ggml, Vulkan; the 2026-10-31 bench binary) on the
# 10.4 s FLEURS clip with and without -nt (no timestamps; the app's decode does not set it).
# Reports whisper_print_timings total - load, 3 runs each.
$b = "$env:USERPROFILE\flow-bench"
$wav = "$b\audio-he\4872784854716589132.wav"
foreach ($nt in @($true, $false)) {
  foreach ($i in 1..3) {
    $args = @('-m', "$b\models\ivrit-turbo-q5_0.bin", '-f', $wav, '-t', '4', '-l', 'he', '-bs', '1', '-bo', '1')
    if ($nt) { $args += '-nt' }
    $err = & "$b\bin\whisper-vk2\whisper-cli.exe" @args 2>&1 | % { "$_" }
    $tot = ($err | Select-String 'total time =\s+([\d.]+)').Matches.Groups[1].Value
    $load = ($err | Select-String 'load time =\s+([\d.]+)').Matches.Groups[1].Value
    $enc = ($err | Select-String 'encode time =\s+([\d.]+)').Matches.Groups[1].Value
    $dec = ($err | Select-String '(decode|batchd) time =\s+([\d.]+)' | % { $_.Matches.Groups[2].Value }) -join '+'
    "nt=$nt run=$i compute_ms=$([double]$tot - [double]$load) encode=$enc decode/batchd=$dec"
  }
}
