# Scenario h (uninstall data): dummy data in the app's data folders, and
# what is there now. Prints key=value lines.
#
#   -Action check              murmur= flowclone= local= present|absent,
#                              installed=True|False, uninstaller=<path>,
#                              install_dir=<path>.
#   -Action seed -Id <run id>  %APPDATA%\Murmur and %APPDATA%\FlowClone get a
#                              dummy prefs.json (onboarded true), a marker file
#                              and (Murmur) a tiny models\ file. Refuses unless
#                              the owner's data is in this run's backup
#                              (appdata.ps1 PENDING = <run id>): the folders
#                              it writes, and the uninstaller then deletes,
#                              must be test data.
#
# Never touches %USERPROFILE%\murmur-e2e\backups.
param(
    [ValidateSet('check', 'seed')][string]$Action = 'check',
    [string]$Id
)
$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Murmur'
$dirs = [ordered]@{
    murmur    = Join-Path $env:APPDATA 'Murmur'
    flowclone = Join-Path $env:APPDATA 'FlowClone'
    local     = Join-Path $env:LOCALAPPDATA 'dev.murmur.windows'
}
$marker = 'e2e-uninstall-data.txt'

switch ($Action) {
    'check' {
        foreach ($k in $dirs.Keys) { "$k=$(if (Test-Path -LiteralPath $dirs[$k]) { 'present' } else { 'absent' })" }
        $installed = Test-Path $key
        "installed=$installed"
        if ($installed) {
            $e = Get-ItemProperty $key
            "uninstaller=$($e.UninstallString.Trim('"'))"
            "install_dir=$($e.InstallLocation.Trim('"'))"
        }
    }
    'seed' {
        $pendingFile = Join-Path $env:USERPROFILE 'murmur-e2e\backups\PENDING'
        $pending = if (Test-Path $pendingFile) { (Get-Content $pendingFile -Raw).Trim() } else { '' }
        if (-not $Id -or $pending -ne $Id) {
            throw "refusing to seed: the owner's data is not in this run's backup (PENDING='$pending', run '$Id')"
        }
        $utf8 = New-Object System.Text.UTF8Encoding $false
        foreach ($k in 'murmur', 'flowclone') {
            $d = $dirs[$k]
            New-Item -ItemType Directory -Force $d | Out-Null
            [IO.File]::WriteAllText((Join-Path $d 'prefs.json'), '{"onboarded":true}', $utf8)
            [IO.File]::WriteAllText((Join-Path $d $marker), "e2e scenario h, run $Id", $utf8)
        }
        $models = Join-Path $dirs.murmur 'models'
        New-Item -ItemType Directory -Force $models | Out-Null
        [IO.File]::WriteAllBytes((Join-Path $models 'e2e-dummy.bin'), [byte[]](1..64))
        "seeded=murmur,flowclone"
    }
}
