#!/usr/bin/env bash
# Replay harness: saved recordings x speech models x cleanup models x prompt
# versions through the real murmur-core pipeline, then the score report.
# See tools/replay/README.md.
#
# usage: tools/replay/run.sh --in DIR [--out DIR] [--asr LIST] [--cleanup LIST]
#          [--prompt LIST] [--expected FILE] [--max-spend USD] [murmur-replay args]
#        tools/replay/run.sh --score OUT_DIR [--expected FILE]   (score only)
#        tools/replay/run.sh cargo <cargo args>   (cargo under the memory rules)
#
# Memory rules (shared box with the home murmur-server): every cargo build and
# every run that loads a local LLM is serial under
# $HOME/.cache/flow-clone-build.lock, as an earlyoom-first (choom 1000),
# nice 10 process; builds are --release -j 2 and need >= 4000 MB available,
# local-model runs need model size + 2000 MB.
#
# env: MURMUR_PARAKEET_DIR, SHERPA_ONNX_LIB_DIR, MURMUR_WHISPER_MODEL,
#      MURMUR_LLM_GGUF + MURMUR_LLM_LIB (local Qwen), CARGO_TARGET_DIR,
#      REPLAY_FEATURES (default parakeet,whisper,llm).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
lock="$HOME/.cache/flow-clone-build.lock"
mkdir -p "$(dirname "$lock")"
avail_mb() { awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo; }

locked() { flock "$lock" choom -n 1000 -- nice -n 10 "$@"; }

cargo_locked() {
  local a; a=$(avail_mb)
  if (( a < 4000 )); then echo "replay: only $a MB available (< 4000), not building" >&2; exit 3; fi
  (cd "$root" && locked cargo "$@")
}

if [[ "${1:-}" == cargo ]]; then
  shift
  cargo_locked "$1" -j 2 "${@:2}"
  exit
fi

if [[ "${1:-}" == --score ]]; then
  exec python3 -I "$here/score.py" --rows "$2" "${@:3}"
fi

in=""; out=""; expected=""; llm=""; pass=()
while (( $# )); do
  case "$1" in
    --in) in="$2"; pass+=("$1" "$2"); shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --expected) expected="$2"; shift 2 ;;
    --llm) llm="$2"; shift 2 ;;
    --cleanup) pass+=("$1" "$2"); [[ ",$2," == *,local,* ]] && llm="${llm:-${MURMUR_LLM_GGUF:?--cleanup local needs MURMUR_LLM_GGUF or --llm}}"; shift 2 ;;
    *) pass+=("$1"); shift ;;
  esac
done
[[ -n "$in" ]] || { echo "usage: run.sh --in CORPUS_DIR [...]" >&2; exit 2; }
# Results stay next to the corpus (never in the repo), one dir per run.
out="${out:-$in/replay/$(date -u +%Y-%m-%dT%H-%M-%SZ)}"
case "$(cd "$(dirname "$out")" 2>/dev/null && pwd || echo)/" in
  "$root"/*) echo "replay: --out must be outside the repo (voice-derived text)" >&2; exit 2 ;;
esac
[[ -z "$expected" && -f "$in/expected.json" ]] && expected="$in/expected.json"

features="${REPLAY_FEATURES:-parakeet,whisper,llm}"
cargo_locked build --release --offline -j 2 -p murmur-replay --features "$features" >&2
bin="${CARGO_TARGET_DIR:-$root/target}/release/murmur-replay"

args=("${pass[@]}" --out "$out")
if [[ -n "$llm" ]]; then
  need=$(( $(stat -c %s "$llm") / 1048576 + 2000 ))
  a=$(avail_mb)
  if (( a < need )); then echo "replay: $a MB available < $need MB for $(basename "$llm")" >&2; exit 3; fi
  args+=(--llm "$llm" --llm-lib "${MURMUR_LLM_LIB:?MURMUR_LLM_LIB (libmurmur_llm.so) for the local LLM}")
  locked "$bin" "${args[@]}"
else
  nice -n 10 "$bin" "${args[@]}"
fi
score=(--rows "$out")
[[ -n "$expected" ]] && score+=(--expected "$expected")
python3 -I "$here/score.py" "${score[@]}" | tee "$out/score.md"
