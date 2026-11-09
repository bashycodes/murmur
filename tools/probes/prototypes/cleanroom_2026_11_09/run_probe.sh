#!/usr/bin/env bash
# Run probe.rs on the current tree; writes JSON lines to $1.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
out="$1"
cp "$here/probe.rs" "$root/crates/murmur-core/tests/zz_probe.rs"
trap 'rm -f "$root/crates/murmur-core/tests/zz_probe.rs"' EXIT
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/murmur-agents-target-cleanroom}"
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"
export PROBE_CASES="$here/cases.json"
cd "$root"
flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
  cargo test -j 2 --offline -p murmur-core --test zz_probe -- --nocapture 2>"$out.log" \
  | grep '^{' >"$out"
wc -l "$out"
