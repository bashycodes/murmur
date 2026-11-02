# Print the app's interesting stderr lines from app_check.ps1's log ($env:TAG, '' by default).
$log = "$env:TEMP\lw-check-$($env:TAG).log"
Get-Content $log | ? { $_ -match 'ivrit|flash attn|use gpu|^\[|error|GPU found|whisper_backend_init|model size' } | % { $_.Substring(0, [Math]::Min(200, $_.Length)) }
