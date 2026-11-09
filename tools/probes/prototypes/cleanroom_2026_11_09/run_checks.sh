#!/usr/bin/env bash
# usage: run_checks.sh test|clippy|gradle  — the branch's pre-commit checks,
# under the shared build lock (AGENTS.md "Builds").
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/murmur-agents-target-cleanroom}"
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"
lock=(flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10)
cd "$root"
case "$1" in
  test) "${lock[@]}" cargo test -j 2 --offline --workspace --exclude murmur-replay --no-fail-fast ;;
  clippy) "${lock[@]}" cargo clippy -j 2 --offline --workspace --exclude murmur-replay --all-targets -- -D warnings ;;
  one) shift; "${lock[@]}" cargo test -j 2 --offline "$@" ;;
  cargo) shift; sub="$1"; shift; "${lock[@]}" cargo "$sub" -j 2 --offline "$@" ;;
  gradle) cd apps/murmur-android && export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}" && "${lock[@]}" ./gradlew --offline --no-daemon --max-workers=2 -Dorg.gradle.jvmargs=-Xmx2g testHybridDebugUnitTest ;;
esac
echo "rc=$?"
