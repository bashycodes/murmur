# Read-only: list the TTS voices (SAPI desktop + OneCore) and Notepad flavour.
Add-Type -AssemblyName System.Speech
"--- SAPI voices"
(New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | ForEach-Object { $_.VoiceInfo.Name + ' | ' + $_.VoiceInfo.Culture }
"--- OneCore voices"
Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Speech_OneCore\Voices\Tokens' -EA 0 | ForEach-Object { (Get-ItemProperty $_.PSPath).'(default)' }
"--- notepad"
Get-AppxPackage Microsoft.WindowsNotepad -EA 0 | Select-Object -Expand Version
Get-Process notepad -EA 0 | Format-Table Id, SessionId, MainWindowTitle -Auto | Out-String
"--- keyboard layouts"
(Get-WinUserLanguageList).LanguageTag
