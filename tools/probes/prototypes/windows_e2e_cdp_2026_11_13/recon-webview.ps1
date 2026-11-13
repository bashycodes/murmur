# Read-only: the app's processes and the WebView2 browser command line
# (shows whether --remote-debugging-port reached it).
Get-CimInstance Win32_Process -Filter "Name='murmur-windows.exe'" | ForEach-Object { "app pid=$($_.ProcessId) parent=$($_.ParentProcessId)" }
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -match 'murmur' -and $_.CommandLine -notmatch '--type=' } |
    ForEach-Object { "webview pid=$($_.ProcessId): " + ($_.CommandLine -replace '--user-data-dir="[^"]*"', '--user-data-dir=<dir>') }
