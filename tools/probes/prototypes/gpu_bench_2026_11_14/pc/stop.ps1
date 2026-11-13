# Stop the llama-server serve.ps1 started for -Tag (by its PID file, so no
# other process is touched). Its sampler loop then ends on its own.
param([Parameter(Mandatory)][string]$Tag)
$f = Join-Path $env:USERPROFILE "murmur-bench\logs\$Tag.pid"
$id = [int](Get-Content $f)
$p = Get-Process -Id $id -EA SilentlyContinue
if ($p -and $p.ProcessName -eq 'llama-server') { Stop-Process -Id $id; "stopped $id" } else { "not running $id" }
