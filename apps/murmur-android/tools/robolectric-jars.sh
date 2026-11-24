#!/usr/bin/env bash
# The Android runtime jars Robolectric loads (org.robolectric:android-all-instrumented, one per
# SDK the unit tests run on), fetched into the local Maven repository before the tests start.
# Robolectric downloads a missing jar itself when its first test starts, outside Gradle and
# without a retry, so one HTTP 429 from Maven Central failed every Robolectric test of a CI run
# (runs 45351240337, 45376226463). Robolectric skips the download when the jar is already in
# the local repository; this script puts it there.
#
# Usage: tools/robolectric-jars.sh key             the CI cache key (Robolectric version + SDKs)
#        tools/robolectric-jars.sh fetch [REPO]    fetch what is missing or damaged
#        tools/robolectric-jars.sh check [REPO]    exit 0 only when every jar is there and intact
#        tools/robolectric-jars.sh status [REPO]   complete=true|false and some=true|false lines
# REPO defaults to ~/.m2/repository (where Robolectric looks). MAVEN_URLS (space-separated)
# overrides the remotes, tried in order for each file.
#
# Maven Central rate-limits CI runners hard (run 45378026028: 429 through every retry), so each
# file is tried on Maven Central and then on Google's mirror of it, in rounds with jittered
# pauses; a file that verifies is kept even when another one fails. Integrity does not depend on
# the remote: the SHA-512 of every jar and pom is pinned below (taken from Maven Central).
#
# The SDKs: each test's @Config(sdk = [...]) plus targetSdk (the default). The jar version per
# SDK is pinned by the Robolectric version (its DefaultSdkProvider); a Robolectric bump or a new
# SDK fails here until the table below names it. After a local test run the right versions are
# the directories under ~/.m2/repository/org/robolectric/android-all-instrumented/, and the
# hashes are in the .sha512 files beside them (or on Maven Central).
set -euo pipefail
app="$(cd "$(dirname "$0")/../app" && pwd)"
read -r -a maven_urls <<< "${MAVEN_URLS:-https://repo1.maven.org/maven2 https://maven-central.storage-download.googleapis.com/maven2}"

robolectric=$(sed -n 's/.*"org\.robolectric:robolectric:\([^"]*\)".*/\1/p' "$app/build.gradle.kts")
target=$(sed -n 's/^ *targetSdk *= *\([0-9]*\).*/\1/p' "$app/build.gradle.kts")
if [ -z "$robolectric" ] || [ -z "$target" ]; then
  echo "robolectric-jars: no Robolectric version or targetSdk in app/build.gradle.kts" >&2; exit 2
fi
sdks=$({ echo "$target"; grep -rhoE 'sdk *= *\[[^]]*\]' "$app/src/test" | grep -oE '[0-9]+' || true; } | sort -un)

# "<android-all-instrumented version> <jar SHA-512> <pom SHA-512>" for Robolectric $1, SDK $2.
pinned() {
  case "$1:$2" in
    4.14.1:34) echo 14-robolectric-10818077-i7 \
      7cdc0b1306e7865f1acd25a363073149654caee5b8e278c5ae37ff4724185724eff39f0b0111800b521e640bb28698d9d2e655134a801569a0f07470e7b5f455 \
      fb97a3028bb1eb4c4b3d1ab9c23c8b6b245842139f76194afc16afe002a212c2f93baff400a83cb6f35e525d0753e5b13aeda172fbdf0f628b25d21aa4a065d6 ;;
    4.14.1:35) echo 15-robolectric-12650502-i7 \
      87bdd0341838f5f630e1ff412ab29cb2f06e0f8cb7751f14679d11b4ee6084873001f8f6a8581ae6e139fa7c9a654b632412827430f984ebecc3c2588674f7e7 \
      2fd37e590a1cc4e661975041a53a307f27e01aa56a4f391325d6644a435d46ce194f1ee9f8acaf8dfd164f8a9fe9b86fe2a0045b515964a3e8d7f02042732c3c ;;
    *) echo "robolectric-jars: no android-all-instrumented version for Robolectric $1, SDK $2: add it to $0" >&2; return 1 ;;
  esac
}

# The files to have, one "<repo-relative path> <SHA-512>" line each: per SDK its pom, then its jar
# (the jar last: Robolectric takes a present jar as complete).
files() {
  local sdk p v jar pom stem
  for sdk in $sdks; do
    p=$(pinned "$robolectric" "$sdk") || return 1
    read -r v jar pom <<< "$p"
    stem="org/robolectric/android-all-instrumented/$v/android-all-instrumented-$v"
    echo "$stem.pom $pom"
    echo "$stem.jar $jar"
  done
}

# $1 has SHA-512 $2, and its .sha512 file (the layout Robolectric writes) says so.
intact() {
  [ -s "$1" ] && [ "$(cat "$1.sha512" 2>/dev/null)" = "$2" ] && [ "$(sha512sum "$1" | cut -d' ' -f1)" = "$2" ]
}

# One pass over the remotes for one file; it lands at $1/$2 only once it verifies.
fetch_one() {
  local repo=$1 path=$2 sha=$3 url tmp
  mkdir -p "$(dirname "$repo/$path")"
  tmp=$(mktemp "$repo/$path.part.XXXXXX")
  for url in "${maven_urls[@]}"; do
    if curl -fsSL --retry 3 --retry-all-errors --retry-max-time 120 --connect-timeout 30 -o "$tmp" "$url/$path" \
      && [ "$(sha512sum "$tmp" | cut -d' ' -f1)" = "$sha" ]; then
      printf '%s' "$sha" > "$repo/$path.sha512"
      mv -f "$tmp" "$repo/$path"
      echo "fetched $path from $url"
      return 0
    fi
    echo "robolectric-jars: $url/$path: failed or wrong SHA-512" >&2
  done
  rm -f "$tmp"
  return 1
}

fetch() {
  local repo=$1 list round path sha missing
  list=$(files)
  for round in 1 2 3 4; do
    missing=0
    while read -r path sha; do
      if intact "$repo/$path" "$sha"; then
        [ "$round" -eq 1 ] && echo "ok      $path"
        continue
      fi
      fetch_one "$repo" "$path" "$sha" || missing=$((missing + 1))
    done <<< "$list"
    [ "$missing" -eq 0 ] && return 0
    if [ "$round" -lt 4 ]; then
      echo "robolectric-jars: $missing file(s) still missing after round $round; retrying" >&2
      sleep $((round * 30 + RANDOM % 30))
    fi
  done
  echo "robolectric-jars: $missing file(s) could not be fetched" >&2
  return 1
}

# How many of the files are present and intact: prints "<intact> <total>".
count() {
  local repo=$1 path sha ok=0 total=0 list
  list=$(files)
  while read -r path sha; do
    total=$((total + 1))
    if intact "$repo/$path" "$sha"; then ok=$((ok + 1)); fi
  done <<< "$list"
  echo "$ok $total"
}

repo="${2:-$HOME/.m2/repository}"
case "${1:-}" in
  key) echo "robolectric-$robolectric-sdk$(echo "$sdks" | paste -sd- -)" ;;
  fetch) fetch "$repo" ;;
  check)
    read -r ok total <<< "$(count "$repo")"
    [ "$ok" -eq "$total" ] || { echo "robolectric-jars: $((total - ok)) of $total files missing or damaged" >&2; exit 1; } ;;
  status)
    read -r ok total <<< "$(count "$repo")"
    echo "complete=$([ "$ok" -eq "$total" ] && echo true || echo false)"
    echo "some=$([ "$ok" -gt 0 ] && echo true || echo false)" ;;
  *) echo "usage: $0 key | fetch [REPO] | check [REPO] | status [REPO]" >&2; exit 2 ;;
esac
