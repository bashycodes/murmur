#!/usr/bin/env bash
# The server images must run on CPUs without AVX-512 (D118): fails if an x86-64
# file we compile ourselves (murmur-server, libmurmur_*.so) has any AVX-512
# instruction. Not for upstream libraries that pick kernels by CPUID
# (onnxruntime, CUDA).
#   tools/checks/no-avx512.sh [--allow-prefix PREFIX]... FILE...
#     exit 1 naming each file that has some, exit 2 when a file cannot be
#     disassembled
#   --allow-prefix PREFIX  don't count instructions in functions whose symbol
#     starts with PREFIX (reported, not failed). Only for vendor code linked
#     statically into our files whose AVX-512 code is CPUID-dispatched, and
#     only where D118 records the evidence (libcublasLt_: the CUDA image's
#     libmurmur_llm_cuda.so). Needs the file's symbol table.
# Every AVX-512 instruction is EVEX-encoded: opcode byte 0x62 after any legacy
# prefixes (in 64-bit mode 0x62 is nothing else; BOUND is 32-bit only). The
# opmask instructions (kmovw, kandw, ...) are VEX-encoded and are caught by
# their %k registers.
set -euo pipefail
usage() { echo "usage: no-avx512.sh [--allow-prefix PREFIX]... FILE..." >&2; exit 2; }
allow=""
while [ $# -gt 0 ]; do
  case "$1" in
    --allow-prefix) [ -n "${2:-}" ] || usage; allow="$allow${allow:+ }$2"; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ $# -gt 0 ] || usage
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
  # Allowed code is told apart by its function symbols: without a symbol
  # table every instruction would sit under one label.
  if [ -n "$allow" ] && objdump -t "$f" 2>/dev/null | grep -q 'no symbols'; then
    echo "no-avx512.sh: $f has no symbol table; --allow-prefix needs one" >&2
    exit 2
  fi
  # "<count> <first function> <allowed count>"; lines are
  # "  addr:\tbytes\tmnemonic operands".
  read -r n first allowed < <(awk -F '\t' -v allow="$allow" '
    BEGIN { np = split(allow, P, " ") }
    /^[0-9a-f]+ <.*>:$/ {
      fn = substr($0, index($0, "<") + 1); sub(/>:$/, "", fn)
      ok = 0; for (j = 1; j <= np; j++) if (index(fn, P[j]) == 1) ok = 1
      next
    }
    NF >= 3 && $1 ~ /^ *[0-9a-f]+:$/ {
      k = split($2, b, " "); i = 1
      while (i <= k && b[i] ~ /^(66|67|f0|f2|f3|2e|36|3e|26|64|65)$/) i++
      if ((i <= k && b[i] == "62") || $3 ~ /%k[0-7]/) {
        if (ok) a++; else if (!n++) first = fn
      }
    }
    END { print n + 0, (first == "" ? "-" : first), a + 0 }' "$tmp/dis")
  note=""
  [ "$allowed" = 0 ] || note=" ($allowed allowed in: $allow)"
  if [ "$n" != 0 ]; then
    echo "FAIL $f: $n AVX-512 instructions (first in $first)$note; see D118 (GGML_NATIVE=OFF)" >&2
    bad=1
  else
    echo "ok   $f: no AVX-512$note"
  fi
done
exit "$bad"
