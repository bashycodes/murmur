# Is a harness step still running on the laptop (murmur-e2e\step.json)?
# Waits up to -WaitSec for it; with -Kill, then stops its process tree.
# Prints step_guard=none|finished|stale|killed|alive (+ step_script=,
# step_pid=, step_waited_s=). Not a step itself (no marker).
param([int]$WaitSec = 90, [switch]$Kill)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
Invoke-StepGuard -WaitSec $WaitSec -Kill:$Kill
