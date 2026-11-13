# Did harness text land in one of the owner's Notepad tabs? Searches
# Notepad's saved tab state for the harness's synthetic phrases and prints
# only file names, times, sizes and which phrase matched: never tab content.
param([string[]]$Phrases = @('quick brown fox', 'weekly report', 'Even he got', 'Even the hot', 'before the meeting', 'Even if I', 'murmur-e2e', 'לקנות לחם', 'המצגת', 'sale'))
$dir = Get-ChildItem "$env:LOCALAPPDATA\Packages" -Directory -Filter 'Microsoft.WindowsNotepad_*' | Select-Object -First 1
$ts = Join-Path $dir.FullName 'LocalState\TabState'
$files = @(Get-ChildItem $ts -File | Sort-Object LastWriteTime)
"tabstate files=$($files.Count)"
foreach ($f in $files) {
    $hits = @()
    try {
        $fs = [IO.File]::Open($f.FullName, 'Open', 'Read', 'ReadWrite')
        $ms = New-Object IO.MemoryStream
        $fs.CopyTo($ms); $fs.Close()
        $b = $ms.ToArray()
        $text = [Text.Encoding]::Unicode.GetString($b) + [Text.Encoding]::UTF8.GetString($b)
        foreach ($p in $Phrases) { if ($text.Contains($p)) { $hits += $p } }
        $size = $b.Length
    } catch { $size = "unreadable" }
    "{0} {1:yyyy-MM-dd HH:mm:ss} bytes={2} matches={3}" -f $f.Name, $f.LastWriteTime, $size, ($hits -join '|')
}
