# Shared by the harness's PowerShell side (dot-sourced).

# Text pads (pad.ps1): the window title is exactly "murmur-e2e-pad <name>".
# Everything that looks for a pad matches the title with Test-PadTitle;
# pads.ps1 (no desktop in the ssh session, so no windows to list) matches
# the process command line with Test-PadCommandLine instead.
$PadTitlePrefix = 'murmur-e2e-pad '
function Pad-Title([string]$name) { $PadTitlePrefix + $name }
function Test-PadTitle([string]$title) { $title -and $title.StartsWith($PadTitlePrefix, [StringComparison]::Ordinal) }
function Test-PadCommandLine([string]$cmd) { $cmd -and $cmd -like '*\murmur-e2e\pad.ps1*' }

# Harness steps (run-step.ps1, stepguard.ps1, lib/stepmark.mjs): step.json
# in the work dir names the state-changing script running now, so a step
# orphaned by a killed ssh can be found, waited for and stopped.
$StepMarker = Join-Path $PSScriptRoot 'step.json'
function Get-ProcStart($p) { try { $p.StartTime.ToUniversalTime().ToString('o') } catch { '' } }
function Read-StepMarker {
    if (-not (Test-Path -LiteralPath $StepMarker)) { return $null }
    try { Get-Content -LiteralPath $StepMarker -Raw | ConvertFrom-Json } catch { [pscustomobject]@{ pid = 0; script = '?' } }
}
# The marker's process, if it is still that same process (not a reused pid).
function Get-StepProcess($m) {
    if (-not $m -or -not $m.pid) { return $null }
    $p = Get-Process -Id ([int]$m.pid) -ErrorAction SilentlyContinue
    if (-not $p) { return $null }
    $s = Get-ProcStart $p
    if ($m.proc_start -and $s -and $s -ne $m.proc_start) { return $null }
    $p
}
function Write-StepMarker([string]$script) {
    $m = [ordered]@{ pid = $PID; script = $script; start = (Get-Date).ToUniversalTime().ToString('o'); proc_start = (Get-ProcStart (Get-Process -Id $PID)) }
    $tmp = "$StepMarker.tmp-$PID"
    [IO.File]::WriteAllText($tmp, ($m | ConvertTo-Json -Compress))
    Move-Item -LiteralPath $tmp -Destination $StepMarker -Force
}
function Remove-StepMarker {
    $m = Read-StepMarker
    if ($m -and [int]$m.pid -eq $PID) { Remove-Item -LiteralPath $StepMarker -Force -ErrorAction SilentlyContinue }
}
# Prints step_guard=none|stale|finished|alive|killed (+ step_script, step_pid,
# step_waited_s): no marker; a dead step's leftover marker (removed); a step
# that ended within -WaitSec; one still running (left alone without -Kill);
# one stopped with its process tree.
function Invoke-StepGuard([int]$WaitSec = 90, [switch]$Kill) {
    $m = Read-StepMarker
    if (-not $m) { 'step_guard=none'; return }
    "step_script=$($m.script)"
    "step_pid=$($m.pid)"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    if (-not (Get-StepProcess $m)) {
        Remove-Item -LiteralPath $StepMarker -Force -ErrorAction SilentlyContinue
        'step_guard=stale'; return
    }
    while ((Get-StepProcess $m) -and $sw.Elapsed.TotalSeconds -lt $WaitSec) { Start-Sleep -Milliseconds 500 }
    "step_waited_s=$([int]$sw.Elapsed.TotalSeconds)"
    if (-not (Get-StepProcess $m)) {
        $now = Read-StepMarker
        if ($now -and [int]$now.pid -eq [int]$m.pid) { Remove-Item -LiteralPath $StepMarker -Force -ErrorAction SilentlyContinue }
        'step_guard=finished'; return
    }
    if (-not $Kill) { 'step_guard=alive'; return }
    & taskkill.exe /T /F /PID ([int]$m.pid) 2>&1 | Out-Null
    for ($i = 0; $i -lt 20 -and (Get-StepProcess $m); $i++) { Start-Sleep -Milliseconds 250 }
    if (Get-StepProcess $m) { 'step_guard=alive'; return }
    Remove-Item -LiteralPath $StepMarker -Force -ErrorAction SilentlyContinue
    'step_guard=killed'
}
