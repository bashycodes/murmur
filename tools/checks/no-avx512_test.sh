#!/usr/bin/env bash
# Tests tools/checks/no-avx512.sh on objects assembled here (binutils `as`):
# AVX2 code passes, each AVX-512 form (zmm, opmask, xmm16+, {evex}) fails.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
check="$here/no-avx512.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

obj() { # obj NAME ASM: assemble one function into $tmp/NAME.o
  printf '.text\n.globl %s\n%s:\n%s\nret\n' "$1" "$1" "$2" | as --64 -o "$tmp/$1.o" -
}
fail=0
expect() { # expect 0|1 NAME
  local rc=0
  "$check" "$tmp/$2.o" >"$tmp/out" 2>&1 || rc=$?
  if [ "$rc" != "$1" ]; then
    echo "FAIL: $2: exit $rc, want $1"; cat "$tmp/out"; fail=1
  else
    echo "ok: $2 (exit $rc)"
  fi
}

obj avx2 'vfmadd231ps %ymm1, %ymm2, %ymm3
vpaddd %ymm15, %ymm14, %ymm13
vcvtph2ps %xmm1, %ymm2'
obj zmm 'vaddps %zmm1, %zmm2, %zmm3'
obj opmask 'kmovw %k1, %eax'
obj high_xmm 'vpaddd %xmm17, %xmm1, %xmm2'
obj evex '{evex} vpaddd %xmm1, %xmm2, %xmm3'

expect 0 avx2
expect 1 zmm
expect 1 opmask
expect 1 high_xmm
expect 1 evex

# The failure names the function.
"$check" "$tmp/zmm.o" >"$tmp/out" 2>&1 || true
grep -q 'first in zmm)' "$tmp/out" || { echo "FAIL: no function name"; cat "$tmp/out"; fail=1; }

exit "$fail"
