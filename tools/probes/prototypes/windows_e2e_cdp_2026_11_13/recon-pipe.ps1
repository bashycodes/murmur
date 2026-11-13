# Probe: does WebView2 connect to WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER?
# Waits for a client on \\.\pipe\$Name, sends one CDP request, logs bytes.
param([string]$Name = 'murmur-e2e-cdp', [int]$WaitSec = 40)
$log = Join-Path $PSScriptRoot 'recon-pipe.log'
"start $(Get-Date -f o)" | Set-Content $log
$s = New-Object System.IO.Pipes.NamedPipeServerStream($Name, [System.IO.Pipes.PipeDirection]::InOut, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::Asynchronous)
$t = $s.WaitForConnectionAsync()
if (-not $t.Wait($WaitSec * 1000)) { "no client" | Add-Content $log; exit }
"connected" | Add-Content $log
$req = [Text.Encoding]::UTF8.GetBytes('{"id":1,"method":"Target.getTargets"}' + [char]0)
$s.Write($req, 0, $req.Length); $s.Flush()
$buf = New-Object byte[] 65536
$rt = $s.ReadAsync($buf, 0, $buf.Length)
if ($rt.Wait(5000)) { "read $($rt.Result): " + [Text.Encoding]::UTF8.GetString($buf, 0, [Math]::Min($rt.Result, 600)) | Add-Content $log } else { "no reply" | Add-Content $log }
$s.Dispose()
