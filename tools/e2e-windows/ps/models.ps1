# Scenario b: a speech model in the TEST profile without a download.
#
#   -Action seed  -Dest <folder> -Files name:size,...
#       Copy the model into <folder> (the app's catalogue `dest`, under
#       %APPDATA%\Murmur\models) from a complete copy already on the laptop,
#       first found of:
#         cache        murmur-e2e\models-cache\<leaf>  (stashed by an earlier run)
#         backup       this run's backup of the owner's models folder
#         owner_prefs  the folder the owner's backed-up prefs.json names
#                      (parakeet_model_dir), read only, never printed
#       Complete = every file there with exactly its catalogue size, the
#       app's own "Installed" test. Copies, never links: the test app must
#       not be able to change the owner's files.
#   -Action stash -Dest <folder> -Files name:size,...
#       Copy a complete <folder> (a model setup downloaded) into the cache,
#       so the next run seeds it instead of downloading again.
#
# Refuses unless the owner's data is in a pending backup (appdata.ps1), so
# the live %APPDATA%\Murmur written here is test data. Prints key=value
# lines; never prints the owner's paths.
param(
    [Parameter(Mandatory)][ValidateSet('seed', 'stash')][string]$Action,
    [Parameter(Mandatory)][string]$Dest,
    [Parameter(Mandatory)][string]$Files
)
$ErrorActionPreference = 'Stop'
$work = Join-Path $env:USERPROFILE 'murmur-e2e'
$pending = Join-Path $work 'backups\PENDING'
$liveModels = Join-Path $env:APPDATA 'Murmur\models'
$cache = Join-Path $work 'models-cache'

if (-not (Test-Path -LiteralPath $pending)) { throw "no pending backup: the live Murmur folder may be the owner's" }
$id = (Get-Content -LiteralPath $pending -Raw).Trim()
$parent = [IO.Path]::GetFullPath((Split-Path -Parent $Dest)).TrimEnd('\')
if ($parent -ne [IO.Path]::GetFullPath($liveModels).TrimEnd('\')) { throw "dest is not in the test profile's models folder" }
$leaf = Split-Path -Leaf $Dest
if ($leaf -notmatch '^[\w.-]+$') { throw "unexpected model folder name" }

$want = @()
foreach ($spec in ($Files -split ',')) {
    $i = $spec.LastIndexOf(':')
    if ($i -le 0) { throw "bad -Files entry: $spec" }
    $name = $spec.Substring(0, $i)
    if ($name -notmatch '^[\w.-]+$') { throw "bad file name: $name" }
    $want += [pscustomobject]@{ name = $name; size = [int64]$spec.Substring($i + 1) }
}

function Test-Complete([string]$dir) {
    if (-not $dir -or -not (Test-Path -LiteralPath $dir -PathType Container)) { return $false }
    foreach ($f in $want) {
        $p = Join-Path $dir $f.name
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { return $false }
        if ((Get-Item -LiteralPath $p).Length -ne $f.size) { return $false }
    }
    $true
}

function Copy-Model([string]$from, [string]$to) {
    New-Item -ItemType Directory -Force -Path $to | Out-Null
    foreach ($f in $want) {
        Copy-Item -LiteralPath (Join-Path $from $f.name) -Destination (Join-Path $to $f.name) -Force
    }
}

switch ($Action) {
    'seed' {
        if (Test-Complete $Dest) { 'seed=already'; return }
        $sources = [ordered]@{
            cache  = Join-Path $cache $leaf
            backup = Join-Path $work "backups\$id\Roaming-Murmur\models\$leaf"
        }
        $prefs = Join-Path $work "backups\$id\Roaming-Murmur\prefs.json"
        if (Test-Path -LiteralPath $prefs) {
            try {
                $d = (Get-Content -LiteralPath $prefs -Raw -Encoding UTF8 | ConvertFrom-Json).parakeet_model_dir
                if ($d -and (Split-Path -Leaf $d) -eq $leaf) { $sources['owner_prefs'] = $d }
            } catch { }
        }
        foreach ($k in $sources.Keys) {
            if (-not (Test-Complete $sources[$k])) { continue }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Copy-Model $sources[$k] $Dest
            if (-not (Test-Complete $Dest)) { throw "copied from $k but the files do not match the catalogue sizes" }
            "seed=copied"
            "source=$k"
            "seconds=$([int]$sw.Elapsed.TotalSeconds)"
            return
        }
        'seed=none'
        "reason=no complete copy of $leaf in: $($sources.Keys -join ', ')"
    }
    'stash' {
        $to = Join-Path $cache $leaf
        if (Test-Complete $to) { 'stash=already'; return }
        if (-not (Test-Complete $Dest)) { 'stash=incomplete'; return }
        Copy-Model $Dest $to
        'stash=copied'
    }
}
