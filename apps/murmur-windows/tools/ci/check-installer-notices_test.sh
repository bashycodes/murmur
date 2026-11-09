#!/usr/bin/env bash
# Test for check-installer-notices.sh (repo-checks; no 7-Zip, no installer):
# a stub 7z "unpacks" X by copying the folder src/<name of X>.d, so an NSIS-like layout
# (files by name) and an MSI-like one (streams plus a cabinet with file ids)
# both pass, and a missing notice or a TTS DLL fails.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
C="$here/check-installer-notices.sh"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
cat > "$T/7z" <<'SH'
#!/usr/bin/env bash
# 7z x -y -oDIR FILE
out="${3#-o}"; mkdir -p "$out"; cp -R "$(dirname "$0")/src/$(basename "$4").d/." "$out/"
SH
chmod +x "$T/7z"
export SEVENZIP="$T/7z"
dll() { printf 'MZ\0\0SherpaOnnxCreateOfflineRecognizer %s\0' "$2" > "$1"; }

# NSIS-like: files by name.
mkdir -p "$T/src/a.exe.d/res"; : > "$T/src/a.exe"; : > "$T/src/a.msi"
for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do cp "$root/$f" "$T/src/a.exe.d/res/$f.txt"; done
dll "$T/src/a.exe.d/sherpa-onnx-c-api.dll" ok
dll "$T/src/a.exe.d/sherpa-onnx-cxx-api.dll" ok
"$C" "$T/src/a.exe" | grep -q '^ok ' || fail "NSIS layout"

# MSI-like: streams, one of them a cabinet whose files have WiX ids.
mkdir -p "$T/src/a.msi.d" "$T/src/cab1.cab.d"
cp "$root/COPYRIGHT" "$T/src/cab1.cab.d/I1a2b"; cp "$root/NOTICE" "$T/src/cab1.cab.d/I3c4d"
cp "$root/THIRD-PARTY-LICENSES" "$T/src/cab1.cab.d/I5e6f"; dll "$T/src/cab1.cab.d/I7a8b" ok
printf 'cab' > "$T/src/a.msi.d/cab1.cab"
dll "$T/src/a.msi.d/Binary.WixUIWixca" ok
"$C" "$T/src/a.msi" > "$T/out" || fail "MSI layout"
grep -q 'ok ' "$T/out" || fail "MSI layout output"
rm "$T/src/cab1.cab.d/I5e6f"
if "$C" "$T/src/a.msi" > /dev/null 2>&1; then fail "MSI without THIRD-PARTY-LICENSES accepted"; fi

# A missing notice fails.
rm "$T/src/a.exe.d/res/THIRD-PARTY-LICENSES.txt"
if "$C" "$T/src/a.exe" > /dev/null 2>&1; then fail "missing THIRD-PARTY-LICENSES accepted"; fi
cp "$root/THIRD-PARTY-LICENSES" "$T/src/a.exe.d/res/THIRD-PARTY-LICENSES.txt"
# A DLL with TTS code fails, whatever its name.
dll "$T/src/a.exe.d/sherpa-onnx-cxx-api.dll" espeak-ng-data
if "$C" "$T/src/a.exe" > /dev/null 2>&1; then fail "TTS DLL accepted"; fi
echo "check-installer-notices: ok"
