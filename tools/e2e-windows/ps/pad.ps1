# The harness's text target: a plain window with one multi-line text box,
# titled Pad-Title <name> (common.ps1). Started by agent.ps1 (pad_open) in the
# owner's interactive session.
#
# Why not Notepad: Windows 11 Notepad opens files as tabs in the window the
# owner already has open, so keys and pasted text could reach one of the
# owner's tabs. This window is ours alone: nothing to save, nothing shared,
# closing it discards the text, and it closes itself after $MaxMinutes.
param(
    [Parameter(Mandatory)][string]$Name,
    [int]$MaxMinutes = 15
)
. (Join-Path $PSScriptRoot 'common.ps1')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$form = New-Object System.Windows.Forms.Form
$form.Text = Pad-Title $Name
$form.Size = New-Object System.Drawing.Size(760, 420)
$form.StartPosition = 'CenterScreen'
$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true
$box.AcceptsReturn = $true
$box.ScrollBars = 'Vertical'
$box.Dock = 'Fill'
$box.Font = New-Object System.Drawing.Font('Segoe UI', 14)
$form.Controls.Add($box)
$form.Add_Shown({ $form.Activate(); $box.Focus() | Out-Null })
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = $MaxMinutes * 60 * 1000
$timer.Add_Tick({ $form.Close() })
$timer.Start()
[System.Windows.Forms.Application]::Run($form)
