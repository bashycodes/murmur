# Paste-race probe runner (see ClipRace.cs). Run in the owner's interactive
# session, e.g. from the e2e-windows folder on the laptop:
#   powershell -File itask.ps1 -Script clip-race.ps1 -Arguments "-Out race.jsonl" -Wait -TimeoutSec 600
# The owner's clipboard text is put back at the end.
param(
    [Parameter(Mandatory)][string]$Out,
    [string]$Strategies = 'd0,d2,d5,d15,d40,d100,settle',
    [int]$Trials = 40
)
$ErrorActionPreference = 'Stop'
try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -Path (Join-Path $PSScriptRoot 'ClipRace.cs') -ReferencedAssemblies System.Windows.Forms
    $lines = [PasteRace.Probe]::Run($Strategies.Split(','), $Trials)
    [IO.File]::WriteAllLines((Join-Path $PSScriptRoot $Out), $lines)
} catch {
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot "$Out.err"), $_.Exception.ToString())
    exit 1
}
