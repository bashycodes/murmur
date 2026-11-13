# Run a script in the owner's interactive desktop session (SSH runs in a
# non-interactive one: no SendInput, no visible windows) through a one-shot
# scheduled task, then delete the task. The process the task started keeps
# running after the task is deleted.
#   -Script <file> [-Arguments "<args>"] [-Wait] [-Elevated]
param(
    [Parameter(Mandatory)][string]$Script,
    [string]$Arguments = '',
    [string]$Name = 'MurmurE2E-' + [IO.Path]::GetFileNameWithoutExtension($Script),
    [switch]$Wait,
    # Default: a normal (non-elevated) token, as when the owner starts the app.
    [switch]$Elevated,
    [int]$TimeoutSec = 60
)
$ErrorActionPreference = 'Stop'
$full = (Resolve-Path $Script).Path
$ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
$psArgs = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$full`" $Arguments"
# conhost --headless (Windows 11) runs the console app with no window at all.
$action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\conhost.exe" -Argument "--headless `"$ps`" $psArgs" -WorkingDirectory (Split-Path $full)
# USERDOMAIN reads WORKGROUP over SSH: use the SID.
$user = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel $(if ($Elevated) { 'Highest' } else { 'Limited' })
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 2) -MultipleInstances IgnoreNew
Unregister-ScheduledTask -TaskName $Name -Confirm:$false -EA SilentlyContinue
Register-ScheduledTask -TaskName $Name -Action $action -Principal $principal -Settings $settings | Out-Null
try {
    Start-ScheduledTask -TaskName $Name
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    # Wait until it has at least started.
    do { Start-Sleep -Milliseconds 200; $t = Get-ScheduledTask -TaskName $Name } while ($t.State -eq 'Queued' -and (Get-Date) -lt $deadline)
    if ($Wait) {
        while ((Get-ScheduledTask -TaskName $Name).State -eq 'Running') {
            if ((Get-Date) -gt $deadline) { Stop-ScheduledTask -TaskName $Name; throw "task $Name timed out" }
            Start-Sleep -Milliseconds 250
        }
        "result=" + (Get-ScheduledTaskInfo -TaskName $Name).LastTaskResult
    } else { 'started' }
} finally {
    Unregister-ScheduledTask -TaskName $Name -Confirm:$false -EA SilentlyContinue
}
