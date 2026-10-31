#!/usr/bin/env bash
# On-device benchmark: push the bundle, start BenchActivity, poll, pull the results
# (tools/probes/android_bench/README.md). Needs the hybrid *debug* APK (BenchActivity is
# debug-only) and $ADB = the platform tools' device bridge command, e.g. "<tool> -s <serial>".
#
#   run_on_device.sh push   [BUNDLE_DIR]        # default $CACHE/android-bench/bundle; --sync: unchanged files are not resent
#   run_on_device.sh start  RUN [EXTRAS...]     # e.g. start r1 --es suite stt --es stt parakeet-en,whisper-base
#   run_on_device.sh status RUN                 # status.json: state running|done|stopped|error, done/total
#   run_on_device.sh wait   RUN                 # poll status every 30 s until it is not "running"
#   run_on_device.sh pull   RUN [DEST]          # default $CACHE/android-bench/results/RUN
#   run_on_device.sh stop                       # stop after the current item
set -euo pipefail
: "${ADB:?set ADB to the device bridge command}"
PKG=dev.flowclone
EXT=/sdcard/Android/data/$PKG/files/bench
CACHE="${CACHE:-$HOME/Claude/research/wispr/.cache}"
act="$PKG/.debug.BenchActivity"
cmd="${1:-}"; shift || true
case "$cmd" in
  push)
    src="${1:-$CACHE/android-bench/bundle}"
    [[ -f "$src/manifest.json" ]] || { echo "no bundle at $src (run prepare_bundle.py)" >&2; exit 2; }
    $ADB shell mkdir -p "$EXT/in"
    $ADB push --sync "$src/." "$EXT/in/"
    ;;
  start)
    run="${1:?RUN id}"; shift
    $ADB shell am start -W -n "$act" --es run "$run" "$@"
    ;;
  status)
    $ADB shell cat "$EXT/out/${1:?RUN id}/status.json"
    ;;
  wait)
    run="${1:?RUN id}"
    while :; do
      s="$($ADB shell cat "$EXT/out/$run/status.json" 2>/dev/null || true)"
      echo "$(date +%T) $(echo "$s" | tr -d '\n ')"
      echo "$s" | grep -q '"state": *"running"' || [[ -z "$s" ]] || break
      sleep 30
    done
    ;;
  pull)
    run="${1:?RUN id}"; dest="${2:-$CACHE/android-bench/results/$run}"
    mkdir -p "$dest"
    $ADB pull "$EXT/out/$run/." "$dest/"
    echo "pulled to $dest"
    ;;
  stop)
    $ADB shell am start -n "$act" --ez stop true
    ;;
  *)
    sed -n '2,13p' "$0"; exit 2 ;;
esac
