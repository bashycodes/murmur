#!/usr/bin/env bash
# Build installable APKs for all three flavours into dist/ (git-ignored) and record their sizes
# and SHA-256 in dist/SHA256SUMS + dist/sizes.txt (copy into INSTALL.md "Live test on phone").
#   debug:   debug-signed (this machine's ~/.android/debug.keystore), installable as is.
#   release: signed when flow.release.* / FLOW_RELEASE_* are set (app/build.gradle.kts), else
#            *-release-unsigned.apk (not installable until signed; INSTALL.md "Release build").
# Native libs must be built first: tools/build-native.sh --flavor local|hybrid|remote.
# The old APKs are deleted before assembling: AGP's incremental packaging otherwise leaves the
# previous libflow_ffi.so's bytes as a hole in the rewritten APK (+4.6 / +9 MB seen after a
# native rebuild), which inflates the file but not its contents.
# Usage: tools/dist.sh [--debug-only]
set -euo pipefail
app="$(cd "$(dirname "$0")/.." && pwd)"
cd "$app"
release=1; [[ "${1:-}" == "--debug-only" ]] && release=0
flavors=(local hybrid remote)
for f in "${flavors[@]}"; do
  [[ -f "app/src/$f/jniLibs/arm64-v8a/libflow_ffi.so" ]] || { echo "missing native libs for $f: tools/build-native.sh --flavor $f" >&2; exit 2; }
done
rm -rf app/build/outputs/apk dist
mkdir -p dist
tasks=(assembleLocalDebug assembleHybridDebug assembleRemoteDebug)
[[ $release == 1 ]] && tasks+=(assembleLocalRelease assembleHybridRelease assembleRemoteRelease)
# Offline by default (this box never downloads); CI sets GRADLE_FLAGS="--no-daemon".
./gradlew ${GRADLE_FLAGS---offline} -q "${tasks[@]}"
for f in "${flavors[@]}"; do
  cp "app/build/outputs/apk/$f/debug/app-$f-debug.apk" "dist/flow-clone-$f-debug.apk"
  tools/probes/check_native_so.sh --flavor "$f" "dist/flow-clone-$f-debug.apk" > "dist/check-$f.txt" 2>&1 \
    || { cat "dist/check-$f.txt"; echo "native gate failed for $f" >&2; exit 1; }
  if [[ $release == 1 ]]; then
    for r in "app/build/outputs/apk/$f/release/"*.apk; do
      cp "$r" "dist/flow-clone-$f-$(basename "$r" | sed -E "s/^app-$f-//")"
    done
  fi
done
(cd dist && sha256sum ./*.apk | sed 's| \./| |' > SHA256SUMS)
(cd dist && for a in ./*.apk; do printf '%s %s %s\n' "$(basename "$a")" "$(stat -c %s "$a")" "$(sha256sum "$a" | cut -d' ' -f1)"; done) > dist/sizes.txt
awk '{ printf "| `%s` | %.1f MB | `%s` |\n", $1, $2/1e6, $3 }' dist/sizes.txt
