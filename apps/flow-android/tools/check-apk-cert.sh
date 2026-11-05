#!/usr/bin/env bash
# Fail unless every given APK is signed by the fixed debug key (D56): its signer certificate's
# SHA-256 must equal the one pinned in apps/flow-android/debug-cert.sha256. CI runs it on the
# dist/ debug APKs when the ANDROID_DEBUG_KEYSTORE_B64 secret was present, so a CI APK
# installs over a locally built one (install -r keeps the app's data).
# Usage: tools/check-apk-cert.sh APK...   (apksigner from $APKSIGNER, PATH or the newest
# $ANDROID_HOME/build-tools)
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
want=$(grep -vE '^\s*(#|$)' "$here/debug-cert.sha256" | head -1 | tr -d '[:space:]')
[ "${#want}" -eq 64 ] || { echo "debug-cert.sha256: no 64-hex fingerprint" >&2; exit 2; }
[ "$#" -gt 0 ] || { echo "usage: $0 APK..." >&2; exit 2; }

signer="${APKSIGNER:-}"
if [ -z "$signer" ]; then
  if [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/build-tools" ]; then
    bt=$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)
    [ -n "$bt" ] && signer="$ANDROID_HOME/build-tools/$bt/apksigner"
  fi
  [ -x "$signer" ] || signer=$(command -v apksigner || true)
fi
[ -n "$signer" ] || { echo "apksigner not found" >&2; exit 2; }

echo "apksigner: $signer ($("$signer" version 2>/dev/null || echo "version unknown"))"
bad=0
for apk in "$@"; do
  raw=$("$signer" verify --print-certs "$apk" 2>&1) && rc=0 || rc=$?
  # Every signer's cert digest, whatever the signer label: "Signer #1 certificate ...", or
  # "Signer (minSdkVersion=…, maxSdkVersion=…) certificate ..." when signers differ by SDK range.
  got=$(printf '%s\n' "$raw" | grep -oE 'certificate SHA-256 digest: [0-9a-fA-F]{64}' \
    | awk '{print tolower($4)}' | sort -u || true)
  if [ "$rc" -eq 0 ] && [ "$got" = "$want" ]; then
    echo "ok    $(basename "$apk")  ${got:0:12}…"
  else
    echo "WRONG $(basename "$apk")  apksigner exit $rc, signer(s) [$(echo $got)], want $want" >&2
    if [ -z "$got" ] || [ "$rc" -ne 0 ]; then
      echo "  apksigner output (first 20 lines):" >&2
      printf '%s\n' "$raw" | head -20 | sed 's/^/  | /' >&2
    fi
    bad=1
  fi
done
exit "$bad"
