# Time since the run lock's last sign of life (lib/lock.mjs): prints
# age_s=<seconds> since the newest of the dir's creation, owner.txt and
# heartbeat, or age_s= (empty) when there is no such dir.
param([Parameter(Mandatory)][string]$Path)
if (Test-Path -LiteralPath $Path) {
    $t = @((Get-Item -LiteralPath $Path).CreationTime)
    foreach ($f in 'owner.txt', 'heartbeat') {
        $i = Get-Item -LiteralPath (Join-Path $Path $f) -ErrorAction SilentlyContinue
        if ($i) { $t += $i.LastWriteTime }
    }
    "age_s=" + [int][Math]::Max(0, ((Get-Date) - ($t | Sort-Object | Select-Object -Last 1)).TotalSeconds)
} else { 'age_s=' }
