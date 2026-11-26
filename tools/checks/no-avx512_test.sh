#!/usr/bin/env bash
# Tests tools/checks/no-avx512.sh on objects assembled here (binutils `as`):
# AVX2 code passes (also with a 0x62 operand byte), every AVX-512 form fails,
# and a file objdump cannot read is an error, not a pass.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
check="$here/no-avx512.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

obj() { # obj NAME ASM: assemble one function into $tmp/NAME.o
  printf '.text\n.globl %s\n%s:\n%s\nret\n' "$1" "$1" "$2" | as --64 -o "$tmp/$1.o" -
}
fail=0
expect() { # expect EXIT FILE
  local rc=0
  "$check" "$2" >"$tmp/out" 2>&1 || rc=$?
  if [ "$rc" != "$1" ]; then
    echo "FAIL: $2: exit $rc, want $1"; cat "$tmp/out"; fail=1
  else
    echo "ok: $(basename "$2") (exit $rc)"
  fi
}

# AVX2 / FMA / F16C, plus 0x62 as an immediate, a displacement and after a prefix.
# shellcheck disable=SC2016  # AT&T `$imm`, not a shell expansion
obj avx2 'vfmadd231ps %ymm1, %ymm2, %ymm3
vpaddd %ymm15, %ymm14, %ymm13
vcvtph2ps %xmm1, %ymm2
vpaddd 0x62(%rax), %ymm1, %ymm2
mov $0x62, %eax
movw $0x6262, %ax
punpckldq %xmm1, %xmm2'
expect 0 "$tmp/avx2.o"

# AVX-512: zmm, opmask, xmm/ymm 16-31, {evex}, and EVEX-only forms on xmm/ymm 0-15.
i=0
while IFS= read -r insn; do
  i=$((i + 1))
  obj "avx512_$i" "$insn"
  expect 1 "$tmp/avx512_$i.o"
done <<'EOF'
vaddps %zmm1, %zmm2, %zmm3
kmovw %k1, %eax
vpaddd %xmm17, %xmm1, %xmm2
{evex} vpaddd %xmm1, %xmm2, %xmm3
vpternlogd $0x96, %ymm1, %ymm2, %ymm3
vprold $3, %xmm1, %xmm2
vpabsq %ymm1, %ymm2
vpermt2d %ymm1, %ymm2, %ymm3
vpmovqd %ymm1, %xmm2
vcvtqq2pd %ymm1, %ymm2
EOF

# The failure names the function.
"$check" "$tmp/avx512_1.o" >"$tmp/out" 2>&1 || true
grep -q 'first in avx512_1)' "$tmp/out" || { echo "FAIL: no function name"; cat "$tmp/out"; fail=1; }

# --allow-prefix: AVX-512 only in functions with the prefix passes (and says
# so); the same plus one other function fails; without the flag it fails; a
# file without symbols is an error.
obj vendor_kernel 'vaddps %zmm1, %zmm2, %zmm3'
printf '.text\n.globl vendor_kernel\nvendor_kernel:\nvaddps %%zmm1, %%zmm2, %%zmm3\nret\n.globl ggml_thing\nggml_thing:\nvpabsq %%ymm1, %%ymm2\nret\n' |
  as --64 -o "$tmp/mixed.o" -
cp "$tmp/vendor_kernel.o" "$tmp/stripped.o"; strip "$tmp/stripped.o"
expect_allow() { # expect_allow EXIT FILE
  local rc=0
  "$check" --allow-prefix vendor_ "$2" >"$tmp/out" 2>&1 || rc=$?
  if [ "$rc" != "$1" ]; then
    echo "FAIL: --allow-prefix $2: exit $rc, want $1"; cat "$tmp/out"; fail=1
  else
    echo "ok: --allow-prefix $(basename "$2") (exit $rc)"
  fi
}
expect_allow 0 "$tmp/vendor_kernel.o"
grep -q '1 allowed in: vendor_' "$tmp/out" || { echo "FAIL: allowed count not reported"; cat "$tmp/out"; fail=1; }
expect 1 "$tmp/vendor_kernel.o"
expect_allow 1 "$tmp/mixed.o"
grep -q 'first in ggml_thing)' "$tmp/out" || { echo "FAIL: mixed: wrong function"; cat "$tmp/out"; fail=1; }
expect_allow 2 "$tmp/stripped.o"

# Not an object file, or missing: an error (exit 2), never "ok".
echo 'not an object' >"$tmp/text.o"
expect 2 "$tmp/text.o"
expect 2 "$tmp/missing.o"

exit "$fail"
