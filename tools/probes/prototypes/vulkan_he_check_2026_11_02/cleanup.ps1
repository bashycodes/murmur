# Remove this probe's temp files on the laptop (no-plugin copy, launchers, logs, screenshots) and
# diff prefs.json against the pre-1.0.0 backup (only keys the app itself may have added should differ).
Remove-Item "$env:TEMP\lw-noplugin" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:TEMP\lw-launch-*.cmd", "$env:TEMP\lw-check-*.log", "$env:TEMP\lw-settings-*" -Force -ErrorAction SilentlyContinue
$p = "$env:APPDATA\Murmur\prefs.json"
$bak = Get-ChildItem "$p.bak-*-pre-1.0.0" | sort LastWriteTime | select -Last 1
$py = "import json,sys;a=json.load(open(sys.argv[1],encoding='utf-8'));b=json.load(open(sys.argv[2],encoding='utf-8'));print('changed keys:',sorted(k for k in set(a)|set(b) if a.get(k)!=b.get(k)))"
& python -c $py $bak.FullName $p
