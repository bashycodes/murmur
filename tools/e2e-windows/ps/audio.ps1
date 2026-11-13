# Audio endpoints on the laptop.
#   -Action list                    endpoints + defaults (one per line)
#   -Action save -File f            write the current default endpoints (and
#                                   whether each is muted) to f. Refuses if f
#                                   exists: it holds the owner's real defaults
#                                   from a run that never restored them.
#   -Action restore -File f [-Keep] put the defaults and mute states saved in f
#                                   back, then delete f (-Keep: leave it, for a
#                                   mid-run restore)
#   -Action mute                    mute the default playback and recording
#                                   endpoints (restore unmutes them)
#   -Action waveout                 waveOut device names
#   -Action setdefault -Flow render|capture -Name substr
#                                   make that endpoint the default (all roles)
param(
    [ValidateSet('list', 'save', 'restore', 'waveout', 'setdefault', 'mute')][string]$Action = 'list',
    [string]$File,
    [ValidateSet('render', 'capture')][string]$Flow,
    [string]$Name,
    [switch]$Keep
)
$ErrorActionPreference = 'Stop'
Add-Type -Path (Join-Path $PSScriptRoot 'AudioHelper.cs')

switch ($Action) {
    'list' { [MurmurE2E.Audio]::List() | ForEach-Object { $_.ToString() } }
    'waveout' { [MurmurE2E.Audio]::WaveOutNames() }
    'save' {
        if (Test-Path -LiteralPath $File) {
            throw "an earlier run's saved audio defaults ($File) were never restored: run 'run.sh restore' first"
        }
        $d = [MurmurE2E.Audio]::List() | Where-Object { $_.DefaultConsole -or $_.DefaultComms } |
            ForEach-Object { [pscustomobject]@{ id = $_.Id; name = $_.Name; flow = $_.Flow; console = $_.DefaultConsole; comms = $_.DefaultComms; muted = [MurmurE2E.Audio]::GetMute($_.Id) } }
        $tmp = "$File.tmp"
        ConvertTo-Json @($d) | Set-Content -Path $tmp -Encoding UTF8
        Move-Item -LiteralPath $tmp -Destination $File
        $d | ForEach-Object { "saved $($_.flow) default: $($_.name) console=$($_.console) comms=$($_.comms) muted=$($_.muted)" }
    }
    'setdefault' {
        $e = @([MurmurE2E.Audio]::List() | Where-Object { $_.Flow -eq $Flow -and $_.Name -like "*$Name*" })
        if ($e.Count -ne 1) { throw "need exactly one $Flow endpoint matching '$Name', found $($e.Count)" }
        [MurmurE2E.Audio]::SetDefault($e[0].Id, $true, $true)
        "default $Flow now: $($e[0].Name)"
    }
    'mute' {
        [MurmurE2E.Audio]::List() | Where-Object DefaultConsole | ForEach-Object {
            [MurmurE2E.Audio]::SetMute($_.Id, $true)
            "muted $($_.Flow): $($_.Name)"
        }
    }
    'restore' {
        if (-not (Test-Path -LiteralPath $File)) { 'audio: nothing saved'; return }
        $want = Get-Content $File -Raw | ConvertFrom-Json
        $now = [MurmurE2E.Audio]::List()
        foreach ($w in $want) {
            $cur = $now | Where-Object Id -eq $w.id
            if (-not $cur) { "skip (gone): $($w.name)"; continue }
            if (($w.console -and -not $cur.DefaultConsole) -or ($w.comms -and -not $cur.DefaultComms)) {
                [MurmurE2E.Audio]::SetDefault($w.id, [bool]$w.console, [bool]$w.comms)
                "restored $($w.flow) default: $($w.name)"
            } else { "unchanged: $($w.name)" }
            if ($null -ne $w.muted -and [MurmurE2E.Audio]::GetMute($w.id) -ne [bool]$w.muted) {
                [MurmurE2E.Audio]::SetMute($w.id, [bool]$w.muted)
                "mute back to $($w.muted): $($w.name)"
            }
        }
        if (-not $Keep) { Remove-Item -LiteralPath $File; 'audio defaults restored' }
    }
}
