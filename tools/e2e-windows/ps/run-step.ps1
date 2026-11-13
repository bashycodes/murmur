# Runs one state-changing harness script as a "step" (lib/stepmark.mjs):
#   run-step.ps1 -Script install.ps1 -ParamsB64 <base64 UTF-8 JSON {Name: value|true}>
# First the guard: an earlier step still alive (orphaned by a killed ssh)
# is waited for up to -WaitSec, then stopped with its process tree. Then
# murmur-e2e\step.json = {pid, script, start, proc_start}, the script with
# its named parameters, and the marker removed in finally.
param(
    [Parameter(Mandatory)][string]$Script,
    [string]$ParamsB64 = '',
    [int]$WaitSec = 90
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
$g = @(Invoke-StepGuard -WaitSec $WaitSec -Kill)
if ($g -notcontains 'step_guard=none') { $g }
if ($g -contains 'step_guard=alive') { throw "an earlier harness step is still running and could not be stopped" }
$params = @{}
if ($ParamsB64) {
    $o = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($ParamsB64)) | ConvertFrom-Json
    foreach ($p in $o.PSObject.Properties) { $params[$p.Name] = $p.Value }
}
Write-StepMarker $Script
$global:LASTEXITCODE = 0
try {
    & (Join-Path $PSScriptRoot $Script) @params
} finally {
    Remove-StepMarker
}
if ($LASTEXITCODE) { exit $LASTEXITCODE }
