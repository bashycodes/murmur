# Runs IN the interactive session (scheduled task, see settings_shot.ps1): raise the app window with
# a second launch, click Settings -> Speech to text through UI Automation, dump the Hebrew row text,
# screenshot the window (kept on the laptop / outside the repo).
param([string]$Exe, [string]$Out)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Start-Process $Exe; Start-Sleep 4
$root = [Windows.Automation.AutomationElement]::RootElement
$cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty, (Get-Process murmur-windows | Select -First 1).Id)
$w = $root.FindAll([Windows.Automation.TreeScope]::Children, $cond) | ? { $_.Current.Name -eq 'Murmur' } | Select -First 1
$res = @()
function Click($name) {
  $all = $w.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $b = $all | ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and $_.Current.Name.Trim() -eq $name } | Select -First 1
  if (-not $b) { $script:res += "no button: $name"; return }
  try { $b.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke(); $script:res += "clicked: $name" } catch { $script:res += "invoke failed ${name}: $_" }
  Start-Sleep 3
}
if (-not $w) { "no window" | Set-Content "$Out.txt"; return }
Click 'Settings'
Click 'Speech to text'
$all = $w.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
$all | % { $_.Current.Name } | ? { $_ -match 'Hebrew|Runs on|graphics chip|ivrit|recommended' } | Select -Unique | % { $res += "text: $_" }
$r = $w.Current.BoundingRectangle
$bmp = New-Object Drawing.Bitmap([int]$r.Width, [int]$r.Height)
$g = [Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $bmp.Size)
$bmp.Save($Out, [Drawing.Imaging.ImageFormat]::Png)
$res += "shot: $Out"
$res | Set-Content -Encoding UTF8 "$Out.txt"
