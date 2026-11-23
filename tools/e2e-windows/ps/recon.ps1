# Read-only look at the laptop: Murmur installs, data dirs,
# processes, installers, VB-Cable. Prints no file contents.
$ErrorActionPreference = 'Continue'
"ps=" + $PSVersionTable.PSVersion.ToString()
"admin=" + ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
"--- data dirs"
Get-ChildItem $env:APPDATA, $env:LOCALAPPDATA -Directory | Where-Object Name -match 'murmur|flow' | ForEach-Object {
    $mb = (Get-ChildItem $_.FullName -Recurse -File -EA 0 | Measure-Object Length -Sum).Sum / 1MB
    '{0}  {1:N0}MB' -f $_.FullName, $mb
}
"--- programs"
Get-ChildItem "$env:LOCALAPPDATA\Programs", $env:ProgramFiles -Directory -EA 0 | Where-Object Name -match 'murmur|flow|vb' | Select-Object -Expand FullName
"--- processes"
Get-Process | Where-Object Name -match 'murmur|flow' | Format-Table Name, Id, SessionId, Path -Auto | Out-String -Width 200
"--- installers"
Get-ChildItem "$env:USERPROFILE\Downloads" -File | Where-Object Name -match 'murmur|flow|vb|cable' | Format-Table Name, Length, LastWriteTime -Auto | Out-String -Width 200
"--- uninstall entries"
Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' -EA 0 |
    Where-Object DisplayName -match 'murmur|flow|vb-audio|cable' |
    Format-List PSPath, DisplayName, DisplayVersion, InstallLocation, UninstallString, QuietUninstallString | Out-String -Width 200
"--- sessions"
query user 2>&1
"--- audio endpoints"
Get-CimInstance Win32_SoundDevice | Select-Object -Expand Name
"--- tasks"
Get-ScheduledTask | Where-Object TaskName -match 'murmur|flow|e2e' | Select-Object -Expand TaskName
