# Stop only this probe's own plugin_bench.py run (matched by its command line), nothing else.
Get-CimInstance Win32_Process | ? { $_.Name -eq 'python.exe' -and $_.CommandLine -match 'plugin_bench\.py' } | % {
  "stopping $($_.ProcessId): $($_.CommandLine.Substring(0, [Math]::Min(120, $_.CommandLine.Length)))"
  Stop-Process -Id $_.ProcessId -Force
}
