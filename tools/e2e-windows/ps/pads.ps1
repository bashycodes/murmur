# Stop any text pad (pad.ps1) a run left behind. Prints pads_stopped=N and
# pads_failed=N, plus one line per pad that could not be stopped.
. (Join-Path $PSScriptRoot 'common.ps1')
$stopped = 0
$failed = 0
Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
    Where-Object { Test-PadCommandLine $_.CommandLine } |
    ForEach-Object {
        $procId = $_.ProcessId
        try {
            Stop-Process -Id $procId -Force -ErrorAction Stop
            $stopped++
        } catch {
            if (Get-Process -Id $procId -ErrorAction SilentlyContinue) {
                $failed++
                "could not stop pad pid ${procId}: $($_.Exception.Message)"
            } else { $stopped++ } # it exited on its own meanwhile
        }
    }
"pads_stopped=$stopped"
"pads_failed=$failed"
