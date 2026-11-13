# Self-test of appdata.ps1's journal on fake folders (murmur-e2e\selftest):
# a backup or restore cut off at any point must still end with the
# "owner's" files in place, and test data must never replace them.
# Run: powershell -File appdata-selftest.ps1   (prints PASS/FAIL per case)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$base = Join-Path $here 'selftest'
$script:fails = 0

function AD([string]$action, [hashtable]$more = @{}) {
    & (Join-Path $here 'appdata.ps1') -Action $action -TestBase $base @more | Out-Null
}
# Same, returning the output lines.
function ADo([string]$action, [hashtable]$more = @{}) {
    @(& (Join-Path $here 'appdata.ps1') -Action $action -TestBase $base @more)
}
function Live([string]$label) {
    switch ($label) {
        'Roaming-Murmur' { Join-Path $base 'Roaming\Murmur' }
        'Roaming-FlowClone' { Join-Path $base 'Roaming\FlowClone' }
        'Local-dev.murmur.windows' { Join-Path $base 'Local\dev.murmur.windows' }
    }
}
function Put([string]$label, [string]$who) {
    $d = Live $label
    New-Item -ItemType Directory -Force -Path $d | Out-Null
    Set-Content (Join-Path $d 'who.txt') $who
}
function Who([string]$label) {
    $f = Join-Path (Live $label) 'who.txt'
    if (Test-Path $f) { (Get-Content $f -Raw).Trim() } else { 'none' }
}
function Fresh {
    if (Test-Path $base) { Remove-Item $base -Recurse -Force }
    Put 'Roaming-Murmur' 'owner'
    Put 'Local-dev.murmur.windows' 'owner'
    # no FlowClone: absent
}
function Journal { Get-Content (Join-Path $base 'backups\t1\manifest.json') -Raw | ConvertFrom-Json }
function SetJournal($m) { $m | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $base 'backups\t1\manifest.json') }
function Check([string]$case, [bool]$ok, [string]$why) {
    if ($ok) { "PASS $case" } else { "FAIL $case : $why"; $script:fails++ }
}
function OwnerBack([string]$case) {
    $m = Who 'Roaming-Murmur'; $l = Who 'Local-dev.murmur.windows'; $f = Who 'Roaming-FlowClone'
    Check $case ($m -eq 'owner' -and $l -eq 'owner' -and $f -eq 'none' -and -not (Test-Path (Join-Path $base 'backups\PENDING'))) "Murmur=$m Local=$l FlowClone=$f"
}

# 1. Normal cycle: backup, wipe, test data everywhere, restore.
Fresh
AD backup @{ Id = 't1' }; AD wipe
'Roaming-Murmur', 'Roaming-FlowClone', 'Local-dev.murmur.windows' | ForEach-Object { Put $_ 'test' }
AD restore
OwnerBack 'normal cycle'

# 2. Restore cut off after moving Roaming-Murmur back, before the journal
#    said so; then restore again. The restored dir must survive.
Fresh
AD backup @{ Id = 't1' }
Put 'Roaming-Murmur' 'test'; Put 'Local-dev.murmur.windows' 'test'
Remove-Item (Live 'Roaming-Murmur') -Recurse -Force
Move-Item (Join-Path $base 'backups\t1\Roaming-Murmur') (Live 'Roaming-Murmur')
$out = ADo restore
OwnerBack 'restore cut off before the journal update'
Check 'its message says an earlier restore moved it back' (($out -join "`n") -match 'Roaming-Murmur \(moved back by an earlier, interrupted restore\)') ($out -join ' | ')

# 3. Same, journal updated (state restored), then restore again.
Fresh
AD backup @{ Id = 't1' }
Put 'Local-dev.murmur.windows' 'test'
Move-Item (Join-Path $base 'backups\t1\Roaming-Murmur') (Live 'Roaming-Murmur')
$m = Journal; $m.states.'Roaming-Murmur' = 'restored'; SetJournal $m
AD restore
OwnerBack 'restore cut off after the journal update'

# 4. A second restore after a full one does nothing.
Fresh
AD backup @{ Id = 't1' }; AD restore; AD restore
OwnerBack 'restore twice'

# 5. Backup cut off: journal says moving, the move never happened.
Fresh
AD backup @{ Id = 't1' }; AD restore     # leaves backups\t1; reuse it as a cut-off journal
Remove-Item (Join-Path $base 'backups\t1') -Recurse -Force
New-Item -ItemType Directory -Path (Join-Path $base 'backups\t1') | Out-Null
SetJournal ([pscustomobject]@{ id = 't1'; was_running = $false; states = [pscustomobject]@{ 'Roaming-Murmur' = 'moving'; 'Roaming-FlowClone' = 'absent'; 'Local-dev.murmur.windows' = 'live' } })
Set-Content (Join-Path $base 'backups\PENDING') 't1'
$wipeRefused = $false
try { AD wipe } catch { $wipeRefused = $true }
Check 'wipe refuses while a dir may be the owner''s' $wipeRefused 'wipe ran'
AD restore
OwnerBack 'backup cut off before the move'

# 6. Backup cut off after the move, before the journal said backed_up.
Fresh
New-Item -ItemType Directory -Force -Path (Join-Path $base 'backups\t1') | Out-Null
SetJournal ([pscustomobject]@{ id = 't1'; was_running = $false; states = [pscustomobject]@{ 'Roaming-Murmur' = 'moving'; 'Roaming-FlowClone' = 'absent'; 'Local-dev.murmur.windows' = 'live' } })
Set-Content (Join-Path $base 'backups\PENDING') 't1'
Move-Item (Live 'Roaming-Murmur') (Join-Path $base 'backups\t1\Roaming-Murmur')
Put 'Roaming-Murmur' 'test'
AD restore
OwnerBack 'backup cut off after the move'

# 7. manifest.json lost mid-swap: restore reads manifest.json.bak.
Fresh
AD backup @{ Id = 't1' }
Put 'Roaming-Murmur' 'test'
Check 'a journal update keeps a .bak' (Test-Path (Join-Path $base 'backups\t1\manifest.json.bak')) 'no manifest.json.bak'
Remove-Item (Join-Path $base 'backups\t1\manifest.json')
AD restore
OwnerBack 'restore from manifest.json.bak'

Remove-Item $base -Recurse -Force
if ($script:fails) { "selftest: $($script:fails) FAILED"; exit 1 } else { 'selftest: all passed' }
