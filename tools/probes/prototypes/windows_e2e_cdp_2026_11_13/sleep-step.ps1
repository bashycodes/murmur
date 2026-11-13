# Probe for the step marker (stepguard-probe.mjs): a harmless step that
# only sleeps, so a timeout leaves it orphaned on the laptop.
param([int]$Sec = 60)
Start-Sleep -Seconds $Sec
'slept'
