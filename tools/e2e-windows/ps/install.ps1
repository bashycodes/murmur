# Silent uninstall of the installed Murmur (if any), then silent install of
# $Installer (a file name in Downloads, or a full path). Prints key=value
# lines the harness parses: exe=, version=, autostarted=.
# -VersionOnly: install nothing, print installer_version= (file version info).
param(
    [Parameter(Mandatory)][string]$Installer,
    [int]$TimeoutSec = 120,
    [switch]$VersionOnly
)
$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Murmur'

function Wait-Until([scriptblock]$cond, [int]$sec, [string]$what) {
    $deadline = (Get-Date).AddSeconds($sec)
    while (-not (& $cond)) {
        if ((Get-Date) -gt $deadline) { throw "timed out waiting for $what" }
        Start-Sleep -Milliseconds 500
    }
}

$path = if (Test-Path $Installer) { (Resolve-Path $Installer).Path } else { Join-Path "$env:USERPROFILE\Downloads" $Installer }
if (-not (Test-Path $path)) { throw "installer not found: $path" }
if ($VersionOnly) {
    $vi = (Get-Item -LiteralPath $path).VersionInfo
    "installer_version=$((@($vi.ProductVersion, $vi.FileVersion) | Where-Object { $_ } | Select-Object -First 1))"
    return
}
$sig = Get-AuthenticodeSignature $path
"installer=$([IO.Path]::GetFileName($path)) signature=$($sig.Status)"

Get-Process murmur-windows -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

if (Test-Path $key) {
    $u = (Get-ItemProperty $key).UninstallString.Trim('"')
    "uninstalling $((Get-ItemProperty $key).DisplayVersion)"
    # NSIS re-launches itself from %TEMP% and the first process exits at
    # once: wait for the registry entry to go, not for the process.
    Start-Process -FilePath $u -ArgumentList '/S' | Out-Null
    Wait-Until { -not (Test-Path $key) } $TimeoutSec 'the uninstaller'
    Start-Sleep -Seconds 1
    'uninstalled'
}

$before = @(Get-Process murmur-windows -EA SilentlyContinue).Count
$p = Start-Process -FilePath $path -ArgumentList '/S' -PassThru
if (-not $p.WaitForExit($TimeoutSec * 1000)) { throw 'installer timed out' }
"installer_exit=$($p.ExitCode)"
Wait-Until { Test-Path $key } 30 'the uninstall entry'
$e = Get-ItemProperty $key
$dir = $e.InstallLocation.Trim('"')
$exe = Get-ChildItem $dir -Filter *.exe | Where-Object Name -notmatch 'uninstall' | Select-Object -First 1
"exe=$($exe.FullName)"
"version=$($e.DisplayVersion)"
Start-Sleep -Seconds 2
$after = @(Get-Process murmur-windows -EA SilentlyContinue)
"autostarted=$([bool]($after.Count -gt $before))"
