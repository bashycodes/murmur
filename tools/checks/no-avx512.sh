#!/usr/bin/env bash
# murmur-server's images must run on any x86-64 CPU with AVX2 (x86-64-v3), not
# only on the AVX-512 machine that built them: 1.8.0's server binary carried
# AVX-512 in whisper.cpp's ggml (GGML_NATIVE on the CI runner) and died with
# SIGILL on a Skylake desktop CPU. Checks the x86-64 files we compile ourselves
# (murmur-server, libmurmur_*.so) for AVX-512 instructions.
#   tools/checks/no-avx512.sh FILE...   (exit 1 naming each file that has them)
# Not for third-party libraries that pick their kernels at run time by CPUID:
# upstream onnxruntime (MLAS) and the CUDA libraries ship AVX-512 paths that
# a CPU without it never runs.
# AVX-512 shows in a disassembly as zmm registers, opmask registers (%k0-%k7),
# xmm/ymm 16-31 (EVEX only), or an explicit {evex} encoding.
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: no-avx512.sh FILE..." >&2; exit 2; }
command -v objdump >/dev/null || { echo "no-avx512.sh: objdump not found (binutils)" >&2; exit 2; }
pattern='%zmm[0-9]|%k[0-7]|%[xy]mm(1[6-9]|2[0-9]|3[01])|[{]evex[}]'
bad=0
for f in "$@"; do
  [ -f "$f" ] || { echo "no-avx512.sh: $f not found" >&2; exit 2; }
  # "<count> <first function>" in one pass.
  read -r n first < <(objdump -d --no-show-raw-insn "$f" | awk -v re="$pattern" '
    /^[0-9a-f]+ <.*>:$/ { fn = substr($2, 2, length($2) - 3) }
    $0 ~ re { if (!n++) first = fn }
    END { print n + 0, (first == "" ? "-" : first) }')
  if [ "$n" != 0 ]; then
    echo "FAIL $f: $n AVX-512 instructions (first in $first); build ggml with GGML_NATIVE=OFF" >&2
    bad=1
  else
    echo "ok   $f: no AVX-512"
  fi
done
exit "$bad"
