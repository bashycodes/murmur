# The harness's hands in the owner's interactive session (started through
# itask.ps1): real key presses (SendInput), WAV playback into the VB-Cable,
# our text pad (pad.ps1) + UI Automation, window list / focus / screenshots.
#
# Protocol: TCP on 127.0.0.1:$Port (reached through an SSH tunnel). One
# JSON object per line in, one JSON line out ({ok:true,...} or
# {ok:false,error}). The agent exits on {"cmd":"quit"} or after
# $IdleMinutes without a request, so it never lingers on the laptop.
param(
    [int]$Port = 9224,
    [int]$IdleMinutes = 10
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
. (Join-Path $here 'common.ps1')
$log = Join-Path $here 'agent.log'
Add-Type -AssemblyName System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -Path (Join-Path $here 'AudioHelper.cs')
Add-Type -Path (Join-Path $here 'InputHelper.cs') -ReferencedAssemblies System.Drawing

function Log([string]$m) { Add-Content -Path $log -Value ("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m) -Encoding UTF8 }

function Win($w) {
    if (-not $w) { return $null }
    [ordered]@{ hwnd = $w.Hwnd; title = $w.Title; class = $w.Class; pid = $w.Pid; process = $w.Process; x = $w.X; y = $w.Y; w = $w.W; h = $w.H }
}

function Get-UiaText([long]$hwnd) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$hwnd)
    $docCond = New-Object System.Windows.Automation.OrCondition(
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document)),
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)))
    $els = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $docCond)
    foreach ($el in $els) {
        $tp = $null
        if ($el.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref]$tp)) {
            return @{ text = $tp.DocumentRange.GetText(-1); control = $el.Current.ClassName; via = 'TextPattern' }
        }
        $vp = $null
        if ($el.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) {
            return @{ text = $vp.Current.Value; control = $el.Current.ClassName; via = 'ValuePattern' }
        }
    }
    # The pad's WinForms text box is not always in the UIA tree: ask it directly.
    $t = [MurmurE2E.Windows]::EditText($hwnd)
    if ($null -ne $t) { return @{ text = $t; control = 'EDIT'; via = 'WM_GETTEXT' } }
    throw "no text control under window $hwnd"
}

# --- NSIS uninstaller dialog (scenario h) ---------------------------------
# Tauri's "Delete the application data" box is on the uninstaller's confirm
# page only: under /S that page never shows, so the box cannot be ticked
# silently. Here the uninstaller runs as a person would start it, and the box
# and the buttons are driven with Win32 messages (MurmurE2E.Dlg): UI
# Automation found the window but not the box (first live run). Matching is
# language-independent: the only check box among the dialog's child windows,
# recursively (the page is an inner #32770), and the outer dialog's button
# id 1, which reads "Uninstall" and later "Close".
$UIA = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
# Top-level dialogs (#32770) of processes started at or after $since, other
# than the app itself: the uninstaller re-launches itself from %TEMP%.
function New-Dialogs([datetime]$since) {
    $c = New-Object System.Windows.Automation.PropertyCondition($UIA::ClassNameProperty, '#32770')
    foreach ($w in $UIA::RootElement.FindAll($TS::Children, $c)) {
        $p = Get-Process -Id $w.Current.ProcessId -ErrorAction SilentlyContinue
        if (-not $p -or $p.ProcessName -eq 'murmur-windows') { continue }
        $st = [datetime]::MinValue
        try { $st = $p.StartTime } catch { }
        if ($st -ge $since.AddSeconds(-2)) { [pscustomobject]@{ hwnd = [long]$w.Current.NativeWindowHandle; pid = $p.Id; process = $p.ProcessName; title = $w.Current.Name } }
    }
}
# One entry per child window (hwnd<parent class id style visible enabled
# "text"), for the errors: says what was there when something is missing.
function Dlg-Tree([long]$top) {
    @([MurmurE2E.Dlg]::Children($top) | ForEach-Object {
        '{0:X}<{1:X} {2} id={3} style=0x{4:X8} vis={5} en={6} "{7}"' -f $_.Hwnd, $_.Parent, $_.Class, $_.Id, $_.Style, [int]$_.Visible, [int]$_.Enabled, $_.Text
    }) -join ' | '
}
function Dlg-CheckBoxes([long]$top) { @([MurmurE2E.Dlg]::Children($top) | Where-Object { [MurmurE2E.Dlg]::IsCheckBox($_) }) }
function Dlg-Button([long]$top, [int]$id) { [MurmurE2E.Dlg]::Children($top) | Where-Object { $_.Parent -eq $top -and $_.Id -eq $id } | Select-Object -First 1 }
function Check-Name([int]$v) { switch ($v) { 0 { 'Off' } 1 { 'On' } 2 { 'Indeterminate' } default { "unreadable($v)" } } }
function Invoke-NsisUninstall($r) {
    $timeout = if ($r.timeoutSec) { [int]$r.timeoutSec } else { 120 }
    $regKey = [string]$r.regKey
    $since = Get-Date
    Start-Process -FilePath ([string]$r.uninstaller) | Out-Null
    $o = [ordered]@{ delete_data = [bool]$r.delete_data }
    # 1. The confirm page: a dialog with exactly one check box and button 1.
    $deadline = (Get-Date).AddSeconds(60)
    $top = [long]0; $cb = $null
    while (-not $cb) {
        if ((Get-Date) -gt $deadline) {
            $dlgs = @(New-Dialogs $since)
            $seen = @($dlgs | ForEach-Object { "$($_.process) `"$($_.title)`" children: $(Dlg-Tree $_.hwnd)" })
            # Leave no uninstaller waiting on the desktop.
            foreach ($d in $dlgs) { Stop-Process -Id $d.pid -Force -ErrorAction SilentlyContinue }
            throw "no uninstaller confirm page with one check box and button 1 within 60 s. Dialogs: $($seen -join ' || ')"
        }
        Start-Sleep -Milliseconds 400
        foreach ($d in @(New-Dialogs $since)) {
            $boxes = Dlg-CheckBoxes $d.hwnd
            if ($boxes.Count -eq 1 -and (Dlg-Button $d.hwnd 1)) { $top = $d.hwnd; $cb = $boxes[0]; $o.process = $d.process; break }
        }
    }
    Start-Sleep -Milliseconds 500 # the page finishes drawing
    $o.checkbox_name = $cb.Text
    $o.checkbox_before = Check-Name ([MurmurE2E.Dlg]::GetCheck($cb.Hwnd))
    # NSIS reads the box with BM_GETCHECK when the page is left.
    if ([bool]$r.delete_data) { [MurmurE2E.Dlg]::SetCheck($cb.Hwnd, $true); Start-Sleep -Milliseconds 300 }
    $o.checkbox_after = Check-Name ([MurmurE2E.Dlg]::GetCheck($cb.Hwnd))
    if ($r.shot) { try { [MurmurE2E.Windows]::Shot($top, [string]$r.shot); $o.shot = $r.shot } catch { $o.shot_error = $_.Exception.Message } }
    if (([bool]$r.delete_data) -ne ($o.checkbox_after -eq 'On')) {
        # Cancel (button 2) rather than uninstall with the wrong choice.
        $cancel = Dlg-Button $top 2
        if ($cancel) { [void][MurmurE2E.Dlg]::Click($top, 2, $cancel.Hwnd) }
        throw "the check box is $($o.checkbox_after) (before: $($o.checkbox_before)), wanted delete_data=$([bool]$r.delete_data); cancelled. An unchanged box means the message was blocked (elevated uninstaller?). Children: $(Dlg-Tree $top)"
    }
    # 2. Uninstall, then wait for the entry to go and the window to close
    #    (press Close when the finished page waits for it).
    $b1 = Dlg-Button $top 1
    $o.uninstall_button = $b1.Text
    [void][MurmurE2E.Dlg]::Click($top, 1, $b1.Hwnd)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $closed = $false
    while ($sw.Elapsed.TotalSeconds -lt $timeout) {
        Start-Sleep -Milliseconds 500
        $gone = -not (Test-Path $regKey)
        if (-not [MurmurE2E.Dlg]::Alive($top)) { if ($gone) { break } else { continue } }
        if ($gone -and -not $closed -and -not (Dlg-CheckBoxes $top).Count) {
            $b = Dlg-Button $top 1
            if ($b -and $b.Enabled) {
                $o.close_button = $b.Text
                [void][MurmurE2E.Dlg]::Click($top, 1, $b.Hwnd)
                $closed = $true
            }
        }
    }
    $o.uninstall_s = [int]$sw.Elapsed.TotalSeconds
    $o.entry_removed = -not (Test-Path $regKey)
    $o.window_closed = -not [MurmurE2E.Dlg]::Alive($top)
    if (-not $o.entry_removed) {
        $tree = if ($o.window_closed) { 'window closed' } else { Dlg-Tree $top }
        throw "the uninstall entry is still there after $timeout s. Children: $tree"
    }
    # The uninstaller's temp copy may still be deleting files for a moment.
    Start-Sleep -Seconds 2
    return $o
}

# The foreground guard of a request: its `guard` hwnd, or 0 (no guard).
function Guard-Of($r) { if ($r.guard) { [long]$r.guard } else { [long]0 } }
function Test-Fg([long]$guard) { $guard -eq 0 -or [MurmurE2E.Windows]::Foreground().Hwnd -eq $guard }
# Our pads: powershell windows whose title is a pad title (common.ps1).
function Get-Pads { [MurmurE2E.Windows]::List('powershell') | Where-Object { Test-PadTitle $_.Title } }

function Invoke-Cmd($r) {
    switch ($r.cmd) {
        'ping' {
            return @{ user = $env:USERNAME; session = (Get-Process -Id $PID).SessionId; desktop_usable = [MurmurE2E.Windows]::InputDesktopUsable(); fg = (Win ([MurmurE2E.Windows]::Foreground())) }
        }
        'hold' { return @{ held_ms = [MurmurE2E.Keys]::Hold([string[]]$r.keys, [int]$r.ms) } }
        'double' {
            # With `guard` (a hwnd), only while that window is in the
            # foreground; if it loses the foreground mid-way the try is
            # aborted and Esc (after the release: Alt+Esc would switch
            # windows) cancels whatever the first tap started.
            $tap2 = if ($null -ne $r.tap2Ms) { [int]$r.tap2Ms } else { [int]$r.tapMs }
            $t = [MurmurE2E.Keys]::DoubleGuarded([string[]]$r.keys, [int]$r.tapMs, [int]$r.gapMs, $tap2, (Guard-Of $r))
            $at = [int]$t[3]
            if ($at -ge 2) { [MurmurE2E.Keys]::Hold([string[]]@('Esc'), 40) | Out-Null }
            $o = @{ tap1_ms = $t[0]; gap_ms = $t[1]; tap2_ms = $t[2]; aborted = ($at -ne 0) }
            if ($at) { $o.aborted_at = @('', 'before the first press', 'before the second press', 'before the release')[$at]; $o.fg_process = ([MurmurE2E.Windows]::Foreground()).Process }
            return $o
        }
        'down' { [MurmurE2E.Keys]::Down([string[]]$r.keys); return @{} }
        'up' { [MurmurE2E.Keys]::Up([string[]]$r.keys); return @{} }
        'type' { [MurmurE2E.Keys]::Type([string]$r.text); return @{} }
        'play' {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $dev = [MurmurE2E.Audio]::PlayWav([string]$r.wav, [string]$r.device)
            return @{ device = $dev; played_ms = $sw.ElapsedMilliseconds }
        }
        'talk' {
            # Hold the push-to-talk chord, play the clip into the cable, keep
            # holding $tailMs, release. With `guard` (a hwnd): nothing is
            # pressed unless that window is in the foreground, and if it lost
            # the foreground by the end, Esc right after the release cancels
            # the dictation (Esc while Alt is held would be Alt+Esc).
            $guard = Guard-Of $r
            if (-not (Test-Fg $guard)) {
                return @{ aborted = $true; aborted_at = 'before the press'; fg_process = ([MurmurE2E.Windows]::Foreground()).Process; held_ms = 0 }
            }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $still = $false
            [MurmurE2E.Keys]::Down([string[]]$r.keys)
            try {
                Start-Sleep -Milliseconds ([int]$r.leadMs)
                $dev = [MurmurE2E.Audio]::PlayWav([string]$r.wav, [string]$r.device)
                Start-Sleep -Milliseconds ([int]$r.tailMs)
                $still = Test-Fg $guard
            } finally { [MurmurE2E.Keys]::Up([string[]]$r.keys) }
            if (-not $still) {
                [MurmurE2E.Keys]::Hold([string[]]@('Esc'), 40) | Out-Null
                return @{ aborted = $true; aborted_at = 'before the release'; fg_process = ([MurmurE2E.Windows]::Foreground()).Process; device = $dev; held_ms = $sw.ElapsedMilliseconds }
            }
            return @{ aborted = $false; device = $dev; held_ms = $sw.ElapsedMilliseconds }
        }
        'windows' { return @{ windows = @([MurmurE2E.Windows]::List([string]$r.process) | ForEach-Object { Win $_ }) } }
        'foreground' { return @{ fg = (Win ([MurmurE2E.Windows]::Foreground())) } }
        'focus' { return @{ focused = [MurmurE2E.Windows]::Focus([long]$r.hwnd); fg = (Win ([MurmurE2E.Windows]::Foreground())) } }
        'close' { [MurmurE2E.Windows]::Close([long]$r.hwnd); return @{} }
        'covered' {
            # Only the covering window's process/class: its title may be the owner's.
            $c = [MurmurE2E.Windows]::CoveredBy([long]$r.hwnd)
            return @{ covered = [bool]$c; by_process = $c.Process; by_class = $c.Class }
        }
        'shot' { [MurmurE2E.Windows]::Shot([long]$r.hwnd, [string]$r.path); return @{ path = $r.path } }
        'pad_open' {
            # Our own text window (pad.ps1), never the owner's Notepad: Win11
            # Notepad would open our file as a tab in the owner's window.
            $title = Pad-Title ([string]$r.name)
            $p = Start-Process powershell.exe -PassThru -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', "`"$(Join-Path $here 'pad.ps1')`"", '-Name', [string]$r.name)
            $deadline = (Get-Date).AddSeconds(20)
            while ((Get-Date) -lt $deadline) {
                $w = Get-Pads | Where-Object { $_.Pid -eq $p.Id -and $_.Title -eq $title } | Select-Object -First 1
                if ($w) { Start-Sleep -Milliseconds 300; return @{ win = (Win $w) } }
                Start-Sleep -Milliseconds 200
            }
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            throw 'pad window did not appear'
        }
        'pad_close' {
            # Close one of our pads (and only a pad).
            $w = Get-Pads | Where-Object { $_.Hwnd -eq [long]$r.hwnd } | Select-Object -First 1
            if ($w) {
                [MurmurE2E.Windows]::Close($w.Hwnd)
                Start-Sleep -Milliseconds 500
                Stop-Process -Id $w.Pid -Force -ErrorAction SilentlyContinue
            }
            return @{ closed = [bool]$w }
        }
        'layout' {
            # Read (no `set`) or switch the input language of a window.
            if ($r.set) { return @{ layout = [MurmurE2E.Windows]::SetLayout([long]$r.hwnd, [string]$r.set) } }
            return @{ layout = [MurmurE2E.Windows]::Layout([long]$r.hwnd) }
        }
        'uia_text' { return (Get-UiaText ([long]$r.hwnd)) }
        'nsis_uninstall' { return (Invoke-NsisUninstall $r) }
        'run_wait' {
            # An installer run in this (interactive) session, as the updater
            # or a person would run it; waits for it to exit.
            $p = Start-Process -FilePath ([Environment]::ExpandEnvironmentVariables([string]$r.exe)) -ArgumentList ([string[]]$r.args) -PassThru
            $sec = if ($r.timeoutSec) { [int]$r.timeoutSec } else { 300 }
            if (-not $p.WaitForExit($sec * 1000)) { throw "$([IO.Path]::GetFileName([string]$r.exe)) did not exit within $sec s" }
            return @{ exit_code = $p.ExitCode }
        }
        default { throw "unknown cmd: $($r.cmd)" }
    }
}

$listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, $Port)
$listener.Start()
Log "listening on $Port (session $((Get-Process -Id $PID).SessionId))"
$utf8 = New-Object System.Text.UTF8Encoding $false
$last = Get-Date
try {
    while ($true) {
        if (-not $listener.Pending()) {
            if (((Get-Date) - $last).TotalMinutes -gt $IdleMinutes) { Log 'idle: exiting'; break }
            Start-Sleep -Milliseconds 20
            continue
        }
        $c = $listener.AcceptTcpClient()
        $last = Get-Date
        $quit = $false
        try {
            $s = $c.GetStream()
            $rd = New-Object System.IO.StreamReader($s, $utf8)
            $wr = New-Object System.IO.StreamWriter($s, $utf8)
            $line = $rd.ReadLine()
            try {
                $req = $line | ConvertFrom-Json
                if ($req.cmd -eq 'quit') { $quit = $true; $res = @{ ok = $true } }
                else {
                    $res = Invoke-Cmd $req
                    if ($null -eq $res) { $res = @{} }
                    $res['ok'] = $true
                }
            } catch {
                $res = @{ ok = $false; error = $_.Exception.Message }
                Log "error in $line : $($_.Exception.Message)"
            }
            $wr.WriteLine(($res | ConvertTo-Json -Depth 8 -Compress))
            $wr.Flush()
        } finally { $c.Close() }
        if ($quit) { Log 'quit'; break }
    }
} finally { $listener.Stop() }
