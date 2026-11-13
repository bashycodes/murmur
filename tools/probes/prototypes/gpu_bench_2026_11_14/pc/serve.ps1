# Start one llama-server (Vulkan, all layers on the GPU) and sample its
# memory once a second until it exits. Runs as the script of a one-shot
# interactive scheduled task (tools/e2e-windows/ps/itask.ps1), so an SSH
# session ending does not kill it. Everything stays in the bench folder.
#
#   serve.ps1 -Model <file in models\> -Tag <name> [-Port 8091] [-Spec ngram|none]
#
# Writes logs\<Tag>.pid, logs\<Tag>-server.{out,err} and logs\<Tag>-mem.csv:
#   t, proc_dedicated, proc_shared, proc_committed (GPU Process Memory,
#   summed over the process's adapter instances), adapter_shared,
#   adapter_dedicated (GPU Adapter Memory, summed over adapters),
#   working_set, private_bytes (the process), os_free (bytes).
param(
    [Parameter(Mandatory)][string]$Model,
    [Parameter(Mandatory)][string]$Tag,
    [int]$Port = 8091,
    [ValidateSet('ngram', 'none')][string]$Spec = 'none',
    # Extra llama-server flags, comma separated (e.g. "-fa,off,--swa-full"):
    # the task launcher passes -File arguments, which do not keep quotes.
    [string]$Extra = ''
)
$ErrorActionPreference = 'Stop'
$d = Join-Path $env:USERPROFILE 'murmur-bench'
$logs = Join-Path $d 'logs'
$exe = Join-Path $d 'llama-b12368\llama-server.exe'
# As the murmur-llm plugin: 4096 context, 128 batch, one sequence, the
# legacy (non-Jinja) chat template path, every layer on the GPU.
$a = @('-m', (Join-Path $d "models\$Model"), '--host', '127.0.0.1', '--port', $Port,
    '-ngl', '99', '-c', '4096', '-b', '128', '-ub', '128', '-np', '1', '--no-jinja',
    '--no-webui', '--metrics')
if ($Spec -eq 'ngram') {
    # Prompt-lookup drafting at 4 tokens, as murmur-server's llm_draft.
    $a += @('--spec-type', 'ngram-simple', '--spec-ngram-simple-size-n', '3',
        '--spec-ngram-simple-size-m', '4', '--spec-draft-n-max', '4')
}
if ($Extra) { $a += $Extra -split ',' }
"$exe $($a -join ' ')" | Set-Content (Join-Path $logs "$Tag-cmd.txt")
$base = (Get-Counter '\GPU Adapter Memory(*)\Shared Usage').CounterSamples | Measure-Object CookedValue -Sum
"baseline_adapter_shared,$($base.Sum)" | Set-Content (Join-Path $logs "$Tag-baseline.txt")
$p = Start-Process -FilePath $exe -ArgumentList $a -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $logs "$Tag-server.out") -RedirectStandardError (Join-Path $logs "$Tag-server.err")
$p.Id | Set-Content (Join-Path $logs "$Tag.pid")
$csv = Join-Path $logs "$Tag-mem.csv"
't,proc_dedicated,proc_shared,proc_committed,adapter_shared,adapter_dedicated,working_set,private_bytes,os_free' | Set-Content $csv
while (-not $p.HasExited) {
    try {
        $s = (Get-Counter "\GPU Process Memory(pid_$($p.Id)_*)\Dedicated Usage", "\GPU Process Memory(pid_$($p.Id)_*)\Shared Usage", "\GPU Process Memory(pid_$($p.Id)_*)\Total Committed", '\GPU Adapter Memory(*)\Shared Usage', '\GPU Adapter Memory(*)\Dedicated Usage' -EA SilentlyContinue).CounterSamples
        $sum = { param($pat) ($s | Where-Object Path -like $pat | Measure-Object CookedValue -Sum).Sum }
        $p.Refresh()
        $free = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory * 1024
        "{0},{1},{2},{3},{4},{5},{6},{7},{8}" -f (Get-Date -Format 'HH:mm:ss'),
            (& $sum '*gpu process memory*dedicated usage'), (& $sum '*gpu process memory*shared usage'),
            (& $sum '*gpu process memory*total committed'), (& $sum '*gpu adapter memory*shared usage'),
            (& $sum '*gpu adapter memory*dedicated usage'), $p.WorkingSet64, $p.PrivateMemorySize64, $free |
            Add-Content $csv
    } catch { "# $($_.Exception.Message)" | Add-Content $csv }
    Start-Sleep -Seconds 1
}
