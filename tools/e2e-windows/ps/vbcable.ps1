# VB-Cable (VB-Audio virtual audio cable) on the laptop: status / install /
# uninstall. The driver pack (official vb-audio.com zip, unpacked) must be in
# $Dir. Install = `VBCABLE_Setup_x64.exe -i -h` (silent). Needs an elevated
# token (the SSH session of an admin account has one).
param(
    [ValidateSet('status', 'install', 'uninstall')][string]$Action = 'status',
    [string]$Dir = "$env:USERPROFILE\murmur-e2e\vbcable",
    [int]$TimeoutSec = 180
)
$ErrorActionPreference = 'Stop'

function Get-CableDevices {
    @(Get-CimInstance Win32_SoundDevice | Where-Object Name -match 'VB-Audio Virtual Cable')
}
function Get-CableEndpoints {
    @(Get-PnpDevice -Class AudioEndpoint -EA SilentlyContinue | Where-Object FriendlyName -match 'CABLE' |
        ForEach-Object { '{0} [{1}]' -f $_.FriendlyName, $_.Status })
}

if ($Action -eq 'status') {
    "driver=" + ((Get-CableDevices | ForEach-Object Name) -join ';')
    "endpoints=" + ((Get-CableEndpoints) -join ';')
    exit 0
}

$setup = Join-Path $Dir 'VBCABLE_Setup_x64.exe'
if (-not (Test-Path $setup)) { throw "missing $setup" }
$sig = Get-AuthenticodeSignature $setup
"setup signature: $($sig.Status) / $($sig.SignerCertificate.Subject)"
if ($sig.Status -ne 'Valid') { throw "setup signature not valid: $($sig.Status)" }

if ($Action -eq 'install') {
    if ((Get-CableDevices).Count -gt 0) { 'already installed'; Get-CableEndpoints; exit 0 }
    # A driver signed by a publisher outside TrustedPublisher makes Windows
    # ask "Would you like to install this device software?". Report the
    # catalog signer so a hang can be explained.
    $cat = Get-AuthenticodeSignature (Join-Path $Dir 'vbaudio_cable64_win10.cat')
    "catalog signature: $($cat.Status) / $($cat.SignerCertificate.Subject)"
    # The driver install makes CABLE the default playback AND recording
    # device (seen on the owner's laptop): save the defaults and put them back.
    $saved = Join-Path $env:TEMP 'murmur-e2e-audio-defaults.json'
    & (Join-Path $PSScriptRoot 'audio.ps1') -Action save -File $saved
    $p = Start-Process -FilePath $setup -ArgumentList '-i', '-h' -WorkingDirectory $Dir -PassThru
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force -EA SilentlyContinue
        throw "setup did not finish in $TimeoutSec s (a confirmation dialog is probably waiting on the desktop)"
    }
    "setup exit=$($p.ExitCode)"
    Start-Sleep -Seconds 3
    & (Join-Path $PSScriptRoot 'audio.ps1') -Action restore -File $saved
    Remove-Item $saved -EA SilentlyContinue
} else {
    $p = Start-Process -FilePath $setup -ArgumentList '-u', '-h' -WorkingDirectory $Dir -PassThru
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force -EA SilentlyContinue; throw 'uninstall timed out' }
    "setup exit=$($p.ExitCode)"
}
Start-Sleep -Seconds 3
"driver=" + ((Get-CableDevices | ForEach-Object Name) -join ';')
"endpoints=" + ((Get-CableEndpoints) -join ';')
