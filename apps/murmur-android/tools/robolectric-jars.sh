#!/usr/bin/env bash
# The Android runtime jars Robolectric loads (org.robolectric:android-all-instrumented, one per
# SDK the unit tests run on), fetched into the local Maven repository before the tests start.
# Robolectric downloads a missing jar itself when its first test starts, outside Gradle and
# without a retry, so one HTTP 429 from Maven Central failed every Robolectric test of a CI run
# (runs 45351240337, 45376226463). Robolectric skips the download when the jar is already in
# the local repository; this script puts it there with retries and checks its SHA-512.
#
# Usage: tools/robolectric-jars.sh key            the CI cache key (Robolectric version + SDKs)
#        tools/robolectric-jars.sh fetch [REPO]   fetch what is missing or damaged
#        tools/robolectric-jars.sh check [REPO]   exit 0 only when every jar is there and intact
# REPO defaults to ~/.m2/repository (where Robolectric looks); MAVEN_URL overrides the remote.
#
# The SDKs: each test's @Config(sdk = [...]) plus targetSdk (the default). The jar version per
# SDK is pinned by the Robolectric version (its DefaultSdkProvider); a Robolectric bump or a new
# SDK fails here until the table below names it. After a local test run the right versions are
# the directories under ~/.m2/repository/org/robolectric/android-all-instrumented/.
set -euo pipefail
app="$(cd "$(dirname "$0")/../app" && pwd)"
maven_url="${MAVEN_URL:-https://repo1.maven.org/maven2}"

robolectric=$(sed -n 's/.*"org\.robolectric:robolectric:\([^"]*\)".*/\1/p' "$app/build.gradle.kts")
target=$(sed -n 's/^ *targetSdk *= *\([0-9]*\).*/\1/p' "$app/build.gradle.kts")
[ -n "$robolectric" ] && [ -n "$target" ] || { echo "robolectric-jars: no Robolectric version or targetSdk in app/build.gradle.kts" >&2; exit 2; }
sdks=$({ echo "$target"; grep -rhoE 'sdk *= *\[[^]]*\]' "$app/src/test" | grep -oE '[0-9]+' || true; } | sort -un)

# android-all-instrumented version for Robolectric $1 and SDK $2.
jar_version() {
  case "$1:$2" in
    4.14.1:34) echo 14-robolectric-10818077-i7 ;;
    4.14.1:35) echo 15-robolectric-12650502-i7 ;;
    *) echo "robolectric-jars: no android-all-instrumented version for Robolectric $1, SDK $2: add it to $0" >&2; return 1 ;;
  esac
}

# Directory and file stem of the jar for SDK $2 in repository $1.
artifact() {
  local v; v=$(jar_version "$robolectric" "$2") || return 1
  echo "$1/org/robolectric/android-all-instrumented/$v/android-all-instrumented-$v"
}

# The file has the SHA-512 its .sha512 file names (Maven's: the hex digest, maybe a file name after it).
intact() {
  [ -s "$1" ] && [ -s "$1.sha512" ] || return 1
  [ "$(sha512sum "$1" | cut -d' ' -f1)" = "$(cut -d' ' -f1 < "$1.sha512" | tr -d '[:space:]')" ]
}

# curl with its own backoff (exponential, honours Retry-After on 429), retried from outside
# with a jittered pause when curl gives up.
download() {
  local url=$1 out=$2 try
  for try in 1 2 3 4; do
    if curl -fsSL --retry 8 --retry-all-errors --retry-max-time 600 --connect-timeout 30 -o "$out" "$url"; then
      return 0
    fi
    [ "$try" -lt 4 ] && sleep $((try * 30 + RANDOM % 30))
  done
  echo "robolectric-jars: could not fetch $url" >&2
  return 1
}

fetch() {
  local repo=$1 sdk stem dir tmp f
  for sdk in $sdks; do
    stem=$(artifact "$repo" "$sdk")
    if intact "$stem.jar" && intact "$stem.pom"; then echo "ok      $(basename "$stem").jar"; continue; fi
    dir=$(dirname "$stem")
    mkdir -p "$dir"
    tmp=$(mktemp -d "$dir/.fetch.XXXXXX")
    for f in jar jar.sha512 pom pom.sha512; do
      download "$maven_url/${stem#"$repo"/}.$f" "$tmp/$(basename "$stem").$f"
    done
    intact "$tmp/$(basename "$stem").jar" && intact "$tmp/$(basename "$stem").pom" \
      || { echo "robolectric-jars: SHA-512 mismatch for $(basename "$stem")" >&2; rm -rf "$tmp"; return 1; }
    # The jar goes in last: Robolectric takes a present jar as complete.
    for f in jar.sha512 pom pom.sha512 jar; do mv -f "$tmp/$(basename "$stem").$f" "$stem.$f"; done
    rmdir "$tmp"
    echo "fetched $(basename "$stem").jar"
  done
}

check() {
  local repo=$1 sdk stem
  for sdk in $sdks; do
    stem=$(artifact "$repo" "$sdk")
    intact "$stem.jar" && intact "$stem.pom" || { echo "missing or damaged: $(basename "$stem").jar" >&2; return 1; }
  done
}

case "${1:-}" in
  key) echo "robolectric-$robolectric-sdk$(echo "$sdks" | paste -sd- -)" ;;
  fetch) fetch "${2:-$HOME/.m2/repository}" ;;
  check) check "${2:-$HOME/.m2/repository}" ;;
  *) echo "usage: $0 key | fetch [REPO] | check [REPO]" >&2; exit 2 ;;
esac
