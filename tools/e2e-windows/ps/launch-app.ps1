# Start Murmur in the interactive session (run through itask.ps1).
# -DebugPort > 0 opens the WebView2 DevTools port (CDP) for the harness;
# 0 = a normal start (used to give the owner their app back).
param(
    [Parameter(Mandatory)][string]$Exe,
    [int]$DebugPort = 9223,
    # Extra NAME=VALUE environment entries (probing only).
    [string[]]$Env = @()
)
$log = Join-Path $PSScriptRoot 'launch.log'
# CreateProcess with an explicit environment (Start-Process goes through
# ShellExecute, and the variable did not reach WebView2 that way).
$si = New-Object System.Diagnostics.ProcessStartInfo $Exe
$si.UseShellExecute = $false
$si.WorkingDirectory = Split-Path $Exe
if ($DebugPort -gt 0) {
    $si.EnvironmentVariables['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = "--remote-debugging-port=$DebugPort"
} else {
    $si.EnvironmentVariables.Remove('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')
}
foreach ($kv in $Env) { $i = $kv.IndexOf('='); $si.EnvironmentVariables[$kv.Substring(0, $i)] = $kv.Substring($i + 1) }
$p = [System.Diagnostics.Process]::Start($si)
Add-Content $log ("{0:o} started {1} pid={2} debugPort={3}" -f (Get-Date), $Exe, $p.Id, $DebugPort)
