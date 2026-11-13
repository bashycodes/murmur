# The owner's Murmur data on the laptop: back it up, wipe the live dirs,
# seed a test profile from the backup, and put the owner's data back.
#
#   -Action status
#   -Action backup  -Id <ts>     stop the app, MOVE the live dirs into
#                                backups\<ts> (same volume = a rename).
#   -Action wipe                 delete the live (test) dirs.
#   -Action seed -Patch <json>   live profile = the backed-up prefs.json with
#                                the keys of <json> merged over it, models
#                                COPIED (a hard link would let the test app
#                                change the owner's file).
#   -Action ownerprefs -Keys k,k a few harmless keys of the backed-up prefs
#   -Action restore              delete the live (test) dirs, move the backup
#                                back, clear PENDING. Safe to run again after
#                                a partial restore.
#
# The journal is backups\<id>\manifest.json plus backups\PENDING. Both are
# written BEFORE anything moves and the manifest is updated after every
# move, so a backup or restore cut off at any point can be finished by
# running restore. Per label, `state` is:
#   absent     the owner had no such dir: a live one is test data
#   moving     about to move (the move is a rename: either side holds it)
#   backed_up  the owner's copy is in the backup; a live one is test data
#   restored   the owner's copy is live again: never touch it
# A live dir is only deleted when its label is `absent`, or `backed_up` /
# `moving` with the owner's copy present in the backup.
#
# Never prints prefs values (the server token is in there, DPAPI-wrapped).
param(
    [ValidateSet('status', 'backup', 'wipe', 'seed', 'restore', 'ownerprefs')][string]$Action = 'status',
    [string]$Id,
    [string]$Patch,
    [string[]]$Keys,
    # Self-test only (appdata-selftest.ps1): fake live dirs and backups under
    # this folder, and the app is never stopped.
    [string]$TestBase
)
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:USERPROFILE 'murmur-e2e\backups'
$appData = $env:APPDATA
$localAppData = $env:LOCALAPPDATA
if ($TestBase) {
    $root = Join-Path $TestBase 'backups'
    $appData = Join-Path $TestBase 'Roaming'
    $localAppData = Join-Path $TestBase 'Local'
}
$pending = Join-Path $root 'PENDING'
# label -> live path. FlowClone is moved too: the renamed app copies an old
# FlowClone folder into Murmur on first start (D83), which would make a
# "fresh" install not fresh.
$dirs = [ordered]@{
    'Roaming-Murmur'           = Join-Path $appData 'Murmur'
    'Roaming-FlowClone'        = Join-Path $appData 'FlowClone'
    'Local-dev.murmur.windows' = Join-Path $localAppData 'dev.murmur.windows'
}

function Stop-App {
    if ($TestBase) { return @() }
    $p = @(Get-Process murmur-windows -EA SilentlyContinue)
    foreach ($x in $p) { Stop-Process -Id $x.Id -Force -EA SilentlyContinue }
    if ($p.Count) { Start-Sleep -Milliseconds 800 }
    # WebView2 children can hold the user-data dir for a moment.
    for ($i = 0; $i -lt 20; $i++) {
        $wv = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
                Where-Object CommandLine -match 'dev\.murmur\.windows')
        if (-not $wv.Count) { break }
        $wv | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }
        Start-Sleep -Milliseconds 300
    }
    return $p
}

function Get-PendingId {
    if (Test-Path $pending) { (Get-Content $pending -Raw).Trim() } else { $null }
}

# backups\<id> and backups\<id>\<label>: the only places the owner's copies live.
function Backup-Dir([string]$id) { Join-Path $root $id }
function Backup-Path([string]$id, [string]$label) { Join-Path (Backup-Dir $id) $label }

# manifest.json, or manifest.json.bak if the main file is missing or torn.
function Read-Journal([string]$id) {
    $m = $null
    foreach ($f in 'manifest.json', 'manifest.json.bak') {
        $path = Join-Path (Backup-Dir $id) $f
        if (-not (Test-Path -LiteralPath $path)) { continue }
        try { $m = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json; if ($m.id) { break } } catch { $m = $null }
    }
    if (-not $m) { throw "backup $id has no readable manifest.json (or .bak)" }
    $states = [ordered]@{}
    foreach ($label in $dirs.Keys) {
        $s = $m.states.$label
        if (-not $s) {
            # Journal from before states existed: listed in `moved` = backed up.
            $s = if (@($m.moved) -contains $label) { 'backed_up' } else { 'unknown' }
        }
        $states[$label] = $s
    }
    $m | Add-Member -NotePropertyName states -NotePropertyValue ([pscustomobject]$states) -Force
    return $m
}

# Write manifest.tmp, then swap it in with File.Replace (the old manifest
# becomes manifest.json.bak): at every instant a full manifest exists.
function Write-Journal($m) {
    $dir = Backup-Dir $m.id
    $tmp = Join-Path $dir 'manifest.tmp'
    $main = Join-Path $dir 'manifest.json'
    $json = $m | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($tmp, $json, (New-Object Text.UTF8Encoding $false))
    if (Test-Path -LiteralPath $main) {
        [IO.File]::Replace($tmp, $main, (Join-Path $dir 'manifest.json.bak'))
    } else {
        [IO.File]::Move($tmp, $main)
    }
}

function Set-State($m, [string]$label, [string]$state) {
    $m.states.$label = $state
    Write-Journal $m
}

# Is the live dir of $label test data (safe to delete)?
function Test-LiveIsTest($m, [string]$label) {
    $s = $m.states.$label
    if ($s -eq 'absent') { return $true }
    if ($s -in 'backed_up', 'moving') { return (Test-Path (Backup-Path $m.id $label)) }
    return $false
}

# Delete the live test dirs; refuse (throw) on any dir that may be the owner's.
function Remove-TestLive($m) {
    foreach ($label in $dirs.Keys) {
        $live = $dirs[$label]
        if (-not (Test-Path $live)) { continue }
        if (-not (Test-LiveIsTest $m $label)) { throw "refusing to delete $label (state $($m.states.$label)): it may be the owner's" }
        Remove-Item -LiteralPath $live -Recurse -Force
        "wiped $label"
    }
}

function Require-Pending {
    $id = Get-PendingId
    if (-not $id) { throw 'no pending backup: refusing to touch the live data (it is the owner''s)' }
    return (Read-Journal $id)
}

switch ($Action) {
    'status' {
        "pending=$(Get-PendingId)"
        foreach ($kv in $dirs.GetEnumerator()) { "$($kv.Key) exists=$(Test-Path $kv.Value)" }
        if (Get-PendingId) {
            $m = Read-Journal (Get-PendingId)
            foreach ($label in $dirs.Keys) { "state $label=$($m.states.$label)" }
        }
        "running=" + (@(Get-Process murmur-windows -EA SilentlyContinue).Count)
        $u = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Murmur'
        if (Test-Path $u) {
            $inst = (Get-ItemProperty $u).InstallLocation.Trim('"')
            "version=" + (Get-ItemProperty $u).DisplayVersion
            "exe=" + (Get-ChildItem $inst -Filter *.exe | Where-Object Name -notmatch 'uninstall' | Select-Object -First 1).FullName
        }
        if (Test-Path $root) { 'backups=' + ((Get-ChildItem $root -Directory | ForEach-Object Name) -join ',') }
    }
    'backup' {
        if (-not $Id) { throw '-Id required' }
        $p = Get-PendingId
        if ($p) { throw "backup $p is still pending: restore it first (appdata.ps1 -Action restore)" }
        $dest = Backup-Dir $Id
        if (Test-Path $dest) { throw "backup folder $Id exists already" }
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        $procs = if ($TestBase) { @() } else { @(Get-Process murmur-windows -EA SilentlyContinue) }
        $exe = ($procs | Select-Object -First 1 | ForEach-Object Path)
        $states = [ordered]@{}
        foreach ($label in $dirs.Keys) { $states[$label] = if (Test-Path $dirs[$label]) { 'live' } else { 'absent' } }
        $m = [pscustomobject]@{ id = $Id; was_running = [bool]$procs.Count; exe = $exe; at = (Get-Date).ToString('o'); states = [pscustomobject]$states }
        # Journal first (with was_running, so restore relaunches the owner's
        # app), then stop the app, then move.
        Write-Journal $m
        Set-Content -Path $pending -Value $Id -Encoding ASCII
        Stop-App | Out-Null
        foreach ($label in $dirs.Keys) {
            if ($m.states.$label -ne 'live') { continue }
            Set-State $m $label 'moving'
            Move-Item -LiteralPath $dirs[$label] -Destination (Backup-Path $Id $label)
            Set-State $m $label 'backed_up'
            "moved $label"
        }
        "backup=$dest"
        "was_running=$([bool]$procs.Count)"
    }
    'wipe' {
        $m = Require-Pending
        Stop-App | Out-Null
        Remove-TestLive $m
    }
    'seed' {
        $m = Require-Pending
        $src = Backup-Path $m.id 'Roaming-Murmur'
        Stop-App | Out-Null
        Remove-TestLive $m
        $live = $dirs['Roaming-Murmur']
        New-Item -ItemType Directory -Force -Path $live | Out-Null
        # The pick-up of an old FlowClone folder has run already (D83).
        Set-Content (Join-Path $live 'old-data-copied.txt') 'e2e seed: not copied' -Encoding UTF8
        $prefs = [ordered]@{}
        $srcPrefs = Join-Path $src 'prefs.json'
        if (Test-Path $srcPrefs) {
            $o = Get-Content $srcPrefs -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($pr in $o.PSObject.Properties) { $prefs[$pr.Name] = $pr.Value }
        }
        if ($Patch) {
            $o = Get-Content $Patch -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($pr in $o.PSObject.Properties) {
                if ($pr.Name -eq 'developer' -and $prefs['developer']) {
                    foreach ($d in $pr.Value.PSObject.Properties) {
                        $prefs['developer'] | Add-Member -NotePropertyName $d.Name -NotePropertyValue $d.Value -Force
                    }
                } else { $prefs[$pr.Name] = $pr.Value }
            }
        }
        $json = [pscustomobject]$prefs | ConvertTo-Json -Depth 32
        [IO.File]::WriteAllText((Join-Path $live 'prefs.json'), $json, (New-Object Text.UTF8Encoding $false))
        "seeded prefs keys=$($prefs.Count) from_owner=$(Test-Path $srcPrefs)"
        # Copies, not hard links: a link shares the file, so a test app
        # rewriting a model in place would change the owner's.
        $models = Join-Path $src 'models'
        $n = 0
        if (Test-Path $models) {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Get-ChildItem $models -Recurse -File | Where-Object Name -notlike '*.part' | ForEach-Object {
                $rel = $_.FullName.Substring($models.Length).TrimStart('\')
                $to = Join-Path (Join-Path $live 'models') $rel
                New-Item -ItemType Directory -Force -Path (Split-Path $to) | Out-Null
                Copy-Item -LiteralPath $_.FullName -Destination $to
                $n++
            }
            "copied model files=$n in $([int]$sw.Elapsed.TotalSeconds) s"
        } else { 'copied model files=0' }
    }
    'ownerprefs' {
        # A few harmless keys of the owner's backed-up prefs, as JSON. Only
        # keys on this list, never the server token or anything key-like.
        $allowed = 'enable_sounds', 'mute_audio_while_dictating', 'microphone', 'shortcuts', 'keyboard_language'
        $m = Require-Pending
        $src = Join-Path (Backup-Path $m.id 'Roaming-Murmur') 'prefs.json'
        $o = [ordered]@{}
        if (Test-Path $src) {
            $p = Get-Content $src -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($k in (($Keys -join ",") -split ",")) { if ($allowed -contains $k) { $o[$k] = $p.$k } }
        }
        'ownerprefs=' + ([pscustomobject]$o | ConvertTo-Json -Depth 4 -Compress)
    }
    'restore' {
        $id = Get-PendingId
        if (-not $id) { 'nothing pending'; return }
        $m = Read-Journal $id
        Stop-App | Out-Null
        foreach ($label in $dirs.Keys) {
            $live = $dirs[$label]
            $saved = Backup-Path $id $label
            $s = $m.states.$label
            if ($s -eq 'absent') {
                if (Test-Path $live) { Remove-Item -LiteralPath $live -Recurse -Force; "removed test $label" }
                continue
            }
            if (Test-Path $saved) {
                # The owner's copy is still in the backup: whatever is live is test data.
                if (Test-Path $live) { Remove-Item -LiteralPath $live -Recurse -Force }
                Move-Item -LiteralPath $saved -Destination $live
                Set-State $m $label 'restored'
                "restored $label"
            } elseif ($s -eq 'restored') {
                # Done by an earlier restore.
            } elseif ($s -in 'backed_up', 'moving') {
                # The journal says the backup holds it, but it is gone from there:
                # an earlier restore moved it back and stopped before updating
                # the journal. The live dir is the owner's again.
                Set-State $m $label 'restored'
                if (Test-Path $live) { "restored $label (moved back by an earlier, interrupted restore)" }
                else { "WARNING $label is neither in the backup nor live" }
            } else {
                # Never moved (backup stopped before it, or state $s): the live
                # dir, if any, is the owner's. Leave it.
                Set-State $m $label 'restored'
                "kept $label (never moved)"
            }
        }
        Remove-Item $pending
        "restored=$id"
        "was_running=$($m.was_running)"
    }
}
