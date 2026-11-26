#!/usr/bin/env bash
# The server images must run on CPUs without AVX-512 (D118): fails if an x86-64
# file we compile ourselves (murmur-server, libmurmur_*.so) has any AVX-512
# instruction. Not for upstream libraries that pick kernels by CPUID
# (onnxruntime, CUDA).
#   tools/checks/no-avx512.sh FILE...   exit 1 naming each file that has some,
#                                       exit 2 when a file cannot be disassembled
# Every AVX-512 instruction is EVEX-encoded: opcode byte 0x62 after any legacy
# prefixes (in 64-bit mode 0x62 is nothing else; BOUND is 32-bit only). The
# opmask instructions (kmovw, kandw, ...) are VEX-encoded and are caught by
# their %k registers.
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: no-avx512.sh FILE..." >&2; exit 2; }
command -v objdump >/dev/null || { echo "no-avx512.sh: objdump not found (binutils)" >&2; exit 2; }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
bad=0
for f in "$@"; do
  # --insn-width=15: all of an instruction's bytes on its own line.
  if ! objdump -d --insn-width=15 "$f" >"$tmp/dis" 2>"$tmp/err"; then
    echo "no-avx512.sh: cannot disassemble $f: $(head -n 1 "$tmp/err")" >&2
    exit 2
  fi
  # "<count> <first function>"; lines are "  addr:\tbytes\tmnemonic operands".
  read -r n first < <(awk -F '\t' '
    /^[0-9a-f]+ <.*>:$/ { fn = substr($0, index($0, "<") + 1); sub(/>:$/, "", fn); next }
    NF >= 3 && $1 ~ /^ *[0-9a-f]+:$/ {
      k = split($2, b, " "); i = 1
      while (i <= k && b[i] ~ /^(66|67|f0|f2|f3|2e|36|3e|26|64|65)$/) i++
      if ((i <= k && b[i] == "62") || $3 ~ /%k[0-7]/) { if (!n++) first = fn }
    }
    END { print n + 0, (first == "" ? "-" : first) }' "$tmp/dis")
  if [ "$n" != 0 ]; then
    echo "FAIL $f: $n AVX-512 instructions (first in $first); see D118 (GGML_NATIVE=OFF)" >&2
    bad=1
  else
    echo "ok   $f: no AVX-512"
  fi
done
exit "$bad"
