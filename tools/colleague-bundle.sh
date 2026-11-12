#!/usr/bin/env bash
# Build the bundle to hand to colleagues: the hybrid Windows installer and
# Android APK (CI builds, fixed debug key D56), murmur-server for Docker
# (docker/: the server's source archive and the image built from it, D77)
# and for Linux x86_64 without Docker (server/; built here: CI does not
# build it), the speech models, the AI cleanup model and an install note
# (docs/colleague-install.md, version filled in; Docker first), plus
# COPYRIGHT.txt, NOTICE.txt and THIRD-PARTY-LICENSES.txt (D80); each model
# zip carries MODEL-LICENSES.txt (tools/bundle/model_notice.py).
#
#   tools/colleague-bundle.sh --source integration|notetaker|BRANCH|COMMIT [options]
#
# --source        what to bundle: a branch on origin (its newest commit) or
#                 a commit. The CI runs are the newest green ones on that
#                 branch at or before the commit; the models come from that
#                 commit's catalogue, so `notetaker` brings the speaker models.
# --out DIR       output folder (default dist/colleague-bundle in the repo;
#                 git-ignored). Downloaded models are kept in DIR/models and
#                 reused by the next run.
# --cache DIR     look for model files here before downloading (repeatable;
#                 any layout: files are found by name and size, then checked
#                 against the pinned SHA-256), e.g. a murmur-server library.
# --llm WHICH     cleanup models: default (Qwen2.5 1.5B, 1.1 GB), all
#                 (+ Llama 3.2 3B for GPUs, Qwen2.5 0.5B; 3.6 GB), none.
# --only LIST     only these model folders (comma-separated ids), e.g. a test.
# --gpu-models    also the speech models only worth it on an NVIDIA GPU
#                 (D84: stock whisper-large-v3-turbo-q5_0, 574 MB); left out
#                 by default.
# --android-run ID, --windows-run ID   use these CI runs instead.
# --allow-stale   accept a CI run older than the commit even when the app's
#                 files changed since (default: refuse).
# --server-target DIR  a cargo release dir that already holds murmur-server
#                 (features whisper,parakeet,tls,llm) and libmurmur_llm.so
#                 built from the same commit: no build.
# --sherpa-lib DIR     libsherpa-onnx-c-api.so + libonnxruntime.so, the no-TTS
#                 build (default $SHERPA_ONNX_LIB_DIR); needed to build and bundled.
# --no-server     leave the host build (server/) out.
# --no-image      leave the Docker image out (no Docker here); the source
#                 archive is still bundled. Without it, Docker must work.
# --no-zip        stop after the folder.
# --no-fetch      do not `git fetch origin` first.
# --dry-run       list what would be fetched or built, with sizes; change nothing.
#
# The server build runs `cargo --offline -j 2` in a detached worktree of the
# commit (DIR/.src/<sha>), target $CARGO_TARGET_DIR (default
# ~/.cache/murmur-bundle-target), under flock/choom/nice, and needs
# 4000 MB of available memory. The image is `docker build` of the bundled
# source archive itself (needs network for crates.io and the sherpa-onnx
# libraries), under the same lock, then `docker save`d with the tags
# murmur-server:<version> and murmur-server:latest.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HERE="$ROOT/tools/bundle"
SOURCE="" OUT="$ROOT/dist/colleague-bundle" LLM=default ONLY="" CACHES=()
ANDROID_RUN="" WINDOWS_RUN="" ALLOW_STALE=0 SERVER_TARGET="" NO_SERVER=0
NO_ZIP=0 NO_FETCH=0 DRY=0 NO_IMAGE=0 SHERPA_LIB="${SHERPA_ONNX_LIB_DIR:-}"
GPU_MODELS=0
# Catalogue speech models bundled only with --gpu-models (D84): on a CPU
# they are too slow to be anyone's pick.
GPU_ONLY=",whisper-large-v3-turbo-q5_0,"

die() { echo "colleague-bundle.sh: $*" >&2; exit 1; }
say() { echo "== $*"; }
while [ $# -gt 0 ]; do
  case "$1" in
    --source) SOURCE="${2:?--source REF}"; shift ;;
    --out) OUT="${2:?--out DIR}"; shift ;;
    --cache) CACHES+=("${2:?--cache DIR}"); shift ;;
    --llm) LLM="${2:?--llm default|all|none}"; shift ;;
    --only) ONLY="${2:?--only LIST}"; shift ;;
    --gpu-models) GPU_MODELS=1 ;;
    --android-run) ANDROID_RUN="${2:?}"; shift ;;
    --windows-run) WINDOWS_RUN="${2:?}"; shift ;;
    --allow-stale) ALLOW_STALE=1 ;;
    --server-target) SERVER_TARGET="${2:?}"; shift ;;
    --sherpa-lib) SHERPA_LIB="${2:?}"; shift ;;
    --no-server) NO_SERVER=1 ;;
    --no-image) NO_IMAGE=1 ;;
    --no-zip) NO_ZIP=1 ;;
    --no-fetch) NO_FETCH=1 ;;
    --dry-run) DRY=1 ;;
    -h|--help) sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done
[ -n "$SOURCE" ] || die "--source integration|notetaker|BRANCH|COMMIT is required"
case "$LLM" in default|all|none) ;; *) die "--llm default|all|none" ;; esac
OUT="$(realpath -m "$OUT")"
for t in git gh python3 curl sha256sum zip; do command -v "$t" >/dev/null || die "$t not found"; done
if [ "$NO_IMAGE" = 0 ]; then
  { command -v docker >/dev/null && docker info >/dev/null 2>&1; } ||
    die "Docker is not available here (needed for the bundle's server image); start it, or pass --no-image"
fi

g() { git -C "$ROOT" "$@"; }
mb() { awk -v b="$1" 'BEGIN { printf "%.0f MB", b / 1e6 }'; }

# ---- source commit ----------------------------------------------------------
[ "$NO_FETCH" = 1 ] || g fetch -q origin
BRANCH=""
if g rev-parse -q --verify "origin/$SOURCE^{commit}" >/dev/null; then
  BRANCH="$SOURCE"; COMMIT=$(g rev-parse "origin/$SOURCE^{commit}")
elif COMMIT=$(g rev-parse -q --verify "$SOURCE^{commit}"); then :
else die "--source $SOURCE: no such branch on origin or commit"; fi
SHA7=${COMMIT:0:7}
VERSION=$(g show "$COMMIT:VERSION" | tr -d '[:space:]')
VARIANT=$(printf '%s' "${BRANCH:-$SHA7}" | tr -c 'A-Za-z0-9._-' '-')
NAME="Murmur-$VERSION-$VARIANT"
BUNDLE="$OUT/$NAME"
say "source $SOURCE = $SHA7, version $VERSION, bundle $NAME"

# ---- models (the commit's catalogue) ---------------------------------------
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
g show "$COMMIT:docs/air-gapped-models.txt" > "$WORK/list.txt"
g show "$COMMIT:apps/murmur-windows/src/model_download.rs" > "$WORK/catalog.rs"
g show "$COMMIT:apps/murmur-windows/src/core_adapter.rs" > "$WORK/core_adapter.rs"
python3 -I "$HERE/models_manifest.py" "$WORK/list.txt" "$WORK/catalog.rs" "$WORK/core_adapter.rs" > "$WORK/all.tsv"
NOTETAKER=0
grep -q $'\tdiarizer\t' "$WORK/all.tsv" && NOTETAKER=1
awk -F'\t' -v llm="$LLM" -v only=",$ONLY," -v gpu="$GPU_MODELS" -v gpuonly="$GPU_ONLY" '
  only != ",," { if (index(only, "," $1 ",")) print; next }
  gpu != 1 && index(gpuonly, "," $1 ",") { next }
  $2 != "llm" { print; next }
  llm == "all" || (llm == "default" && $1 == "qwen2.5-1.5b-instruct-q4_k_m") { print }
' "$WORK/all.tsv" > "$WORK/models.tsv"
[ -s "$WORK/models.tsv" ] || die "no models selected (--only $ONLY?)"
if [ -n "$ONLY" ]; then
  for f in ${ONLY//,/ }; do cut -f1 "$WORK/models.tsv" | grep -qx "$f" || die "--only: no model folder $f"; done
fi
MODELS="$OUT/models"

# Where a model file lives in the bundle's model tree (diarizer: top level).
mpath() { if [ "$2" = diarizer ]; then echo "$1/$3"; else echo "library/$1/$3"; fi; }
verified() { [ -f "$1" ] && [ "$(stat -c %s "$1")" = "$2" ] && [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$3" ]; }
# A cache file with this name and size (not hashed yet), if any.
cached() {
  local c
  for c in "${CACHES[@]}"; do
    [ -d "$c" ] && find -L "$c" -type f -name "$1" -size "${2}c" -print 2>/dev/null
  done
}

# ---- CI runs ----------------------------------------------------------------
# find_run WORKFLOW PATHS...: prints the id of the newest green run on BRANCH
# at or before COMMIT whose app files are unchanged since (or --allow-stale).
# Runs in a command substitution: on failure it prints why and returns 1,
# and the caller stops (a `die` here would only end the subshell).
find_run() {
  local wf=$1 id head
  shift
  local -a runs
  if [ -n "$BRANCH" ]; then
    mapfile -t runs < <(gh run list -w "$wf.yml" -b "$BRANCH" -s success -L 40 --json databaseId,headSha -q '.[] | "\(.databaseId) \(.headSha)"')
  else
    mapfile -t runs < <(gh run list -w "$wf.yml" -c "$COMMIT" -s success -L 5 --json databaseId,headSha -q '.[] | "\(.databaseId) \(.headSha)"')
  fi
  for r in "${runs[@]}"; do
    read -r id head <<<"$r"
    g cat-file -e "$head^{commit}" 2>/dev/null || continue
    g merge-base --is-ancestor "$head" "$COMMIT" || continue
    if [ "$head" != "$COMMIT" ] && ! g diff --quiet "$head" "$COMMIT" -- "$@"; then
      if [ "$ALLOW_STALE" != 1 ]; then
        echo "colleague-bundle.sh: $wf: newest green run $id is at ${head:0:7}, and $* changed since (wait for CI on $SHA7, or --allow-stale)" >&2
        return 1
      fi
      echo "warning: $wf run $id is at ${head:0:7}; app files changed since (--allow-stale)" >&2
    fi
    echo "$id"
    return 0
  done
  echo "colleague-bundle.sh: $wf: no green run on ${BRANCH:-$SHA7} at or before $SHA7" >&2
  return 1
}
APP_PATHS=(crates Cargo.lock VERSION)
A_RUN="$ANDROID_RUN" W_RUN="$WINDOWS_RUN"
[ -n "$A_RUN" ] || A_RUN=$(find_run android-build apps/murmur-android "${APP_PATHS[@]}") || exit 1
[ -n "$W_RUN" ] || W_RUN=$(find_run windows-build apps/murmur-windows "${APP_PATHS[@]}") || exit 1
artifact() { # RUN PREFIX -> "name<TAB>bytes"
  gh api "repos/{owner}/{repo}/actions/runs/$1/artifacts" \
    -q ".artifacts[] | select(.expired | not) | select(.name | startswith(\"$2\")) | \"\(.name)\t\(.size_in_bytes)\"" | head -1
}
IFS=$'\t' read -r A_ART A_BYTES < <(artifact "$A_RUN" murmur-android-debug-apks-) || die "android run $A_RUN: no APK artifact (expired?)"
IFS=$'\t' read -r W_ART W_BYTES < <(artifact "$W_RUN" murmur-windows-hybrid-) || die "windows run $W_RUN: no hybrid artifact (expired?)"
APK="murmur-android-hybrid-$VERSION-debug.apk"
WIN_EXE="Murmur_${VERSION}_x64-setup_hybrid.exe"
MODELS_ZIP="$NAME-speech-models.zip"
CLEANUP_ZIP="$NAME-cleanup-model.zip"
# Docker: the server's source (exactly the files `docker build` needs, plus
# its docs) and the image built from it, in the bundle's docker/ folder.
SOURCE_TGZ="murmur-server-source-$VERSION.tar.gz"
IMAGE_TAR="murmur-server-image-$VERSION.tar"
SRC_DIR="murmur-server-source-$VERSION"
SRC_PATHS=(Cargo.toml Cargo.lock VERSION COPYRIGHT NOTICE THIRD-PARTY-LICENSES .dockerignore crates deploy README.md
  docs/server-setup.md docs/container.md docs/server-advanced.md)
g cat-file -e "$COMMIT:deploy/docker/Dockerfile" 2>/dev/null || die "$SHA7 has no deploy/docker/Dockerfile"
have=()
for p in "${SRC_PATHS[@]}"; do
  if g cat-file -e "$COMMIT:$p" 2>/dev/null; then have+=("$p"); fi   # older commits lack some docs
done
SRC_PATHS=("${have[@]}")

# ---- dry run ----------------------------------------------------------------
if [ "$DRY" = 1 ]; then
  say "CI artifacts (zip sizes)"
  printf '  android-build run %s  %s  %s -> android/%s\n' "$A_RUN" "$A_ART" "$(mb "$A_BYTES")" "$APK"
  printf '  windows-build run %s  %s  %s -> windows/%s\n' "$W_RUN" "$W_ART" "$(mb "$W_BYTES")" "$WIN_EXE"
  say "models (pinned, SHA-256 checked)"
  total=0 fetch=0
  while IFS=$'\t' read -r dir kind _ file size sha _ _; do
    p=$(mpath "$dir" "$kind" "$file")
    if [ -f "$MODELS/$p" ] && [ "$(stat -c %s "$MODELS/$p")" = "$size" ]; then st="have"
    elif [ -n "$(cached "$file" "$size" | head -1)" ]; then st="cache"
    else st="download"; fetch=$((fetch + size)); fi
    total=$((total + size))
    printf '  %-8s %-8s %10s  %s\n' "$st" "$kind" "$(mb "$size")" "$p"
  done < "$WORK/models.tsv"
  echo "  models: $(wc -l < "$WORK/models.tsv") files, $(mb "$total") in all, $(mb "$fetch") to download"
  say "murmur-server"
  if [ "$NO_SERVER" = 1 ]; then echo "  left out (--no-server)"
  elif [ -n "$SERVER_TARGET" ]; then echo "  from $SERVER_TARGET (no build)"
  else echo "  build in $OUT/.src/$SHA7 (cargo --offline -j 2, release, features whisper,parakeet,tls,llm + murmur-llm); sherpa libs: ${SHERPA_LIB:-MISSING (--sherpa-lib)}"; fi
  say "murmur-server for Docker"
  echo "  docker/$SOURCE_TGZ: $SHA7:{${SRC_PATHS[*]}} ($(mb "$(g archive --format=tar "$COMMIT" -- "${SRC_PATHS[@]}" | wc -c)") uncompressed)"
  if [ "$NO_IMAGE" = 1 ]; then echo "  docker/$IMAGE_TAR: left out (--no-image)"
  else echo "  docker/$IMAGE_TAR: docker build of that archive (CARGO_BUILD_JOBS=2, MURMUR_GIT_SHA=$SHA7), docker save murmur-server:$VERSION + :latest"; fi
  echo "  notetaker speaker models: $([ "$NOTETAKER" = 1 ] && echo yes || echo "no (not in this commit's catalogue)")"
  say "would write $BUNDLE/ and $OUT/{$NAME.zip,$MODELS_ZIP$([ "$LLM" != none ] && echo ",$CLEANUP_ZIP")}"
  exit 0
fi

mkdir -p "$OUT" "$MODELS"
[ ! -e "$BUNDLE" ] || die "$BUNDLE exists: delete it first"
mkdir -p "$BUNDLE/windows" "$BUNDLE/android"
# Rights and third-party licences (D80), from the bundled commit.
for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do
  g show "$COMMIT:$f" > "$BUNDLE/$f.txt" 2>/dev/null || { rm -f "$BUNDLE/$f.txt"; die "$SHA7 has no $f"; }
done

# ---- apps -------------------------------------------------------------------
say "CI artifacts"
gh run download "$A_RUN" -n "$A_ART" -D "$WORK/android"
gh run download "$W_RUN" -n "$W_ART" -D "$WORK/windows"
[ -f "$WORK/android/murmur-hybrid-debug.apk" ] || die "no murmur-hybrid-debug.apk in $A_ART"
[ -f "$WORK/windows/$WIN_EXE" ] || die "no $WIN_EXE in $W_ART (version mismatch?)"
cp "$WORK/android/murmur-hybrid-debug.apk" "$BUNDLE/android/$APK"
cp "$WORK/windows/$WIN_EXE" "$BUNDLE/windows/$WIN_EXE"
APKSIGNER="${APKSIGNER:-}"
if [ -z "$APKSIGNER" ] && [ -x "${ANDROID_HOME:-$HOME/Android/Sdk}/build-tools/36.0.0/apksigner" ]; then
  APKSIGNER="${ANDROID_HOME:-$HOME/Android/Sdk}/build-tools/36.0.0/apksigner"
fi
if [ -n "$APKSIGNER" ] || command -v apksigner >/dev/null; then
  APKSIGNER="$APKSIGNER" "$ROOT/apps/murmur-android/tools/check-apk-cert.sh" "$BUNDLE/android/$APK" \
    || die "the APK is not signed with the fixed debug key (D56)"
else
  echo "warning: apksigner not found; the APK's key (D56) is not checked here (CI checks it)" >&2
fi

# ---- models -----------------------------------------------------------------
say "models"
while IFS=$'\t' read -r dir kind engine file size sha url title; do
  p=$(mpath "$dir" "$kind" "$file")
  dst="$MODELS/$p"
  mkdir -p "$(dirname "$dst")"
  if verified "$dst" "$size" "$sha"; then
    echo "ok        $p"
  else
    got=""
    while IFS= read -r c; do
      if verified "$c" "$size" "$sha"; then got="$c"; break; fi
    done < <(cached "$file" "$size")
    if [ -n "$got" ]; then
      cp -f "$got" "$dst.part" && mv -f "$dst.part" "$dst"
      echo "cache     $p"
    else
      curl -fL --retry 3 -C - -o "$dst.part" "$url" </dev/null
      verified "$dst.part" "$size" "$sha" || { rm -f "$dst.part"; die "$p: download does not match the pinned SHA-256"; }
      mv -f "$dst.part" "$dst"
      echo "download  $p"
    fi
  fi
  if [ "$kind" != diarizer ]; then
    id=""; [ "$kind" = lid ] || id="catalogue_id = \"$dir\""
    printf 'kind = "%s"\ntitle = "%s"\nengine = "%s"\n%s\n' "$kind" "$title" "$engine" "$id" > "$MODELS/library/$dir/model.toml"
  fi
done < "$WORK/models.tsv"

# ---- murmur-server smoke test -------------------------------------------------
# smoke_server DIR: start the bundled server (DIR = the bundle's server/) on
# a free loopback port with the bundle's config, pointed at the models
# fetched above, and wait for "ready". Model loading dlopens
# lib/libmurmur_llm.so with the cleanup model, so a broken plugin or a
# missing library fails here. Without the cleanup or parakeet model
# (--llm none, --only) it only checks that the binary starts.
smoke_server() {
  local S=$1 lib="$MODELS/library" d="$WORK/smoke" port pid ok=0 i
  local llm="$lib/qwen2.5-1.5b-instruct-q4_k_m/qwen2.5-1.5b-instruct-q4_k_m.gguf"
  mkdir -p "$d/state"
  MURMUR_SERVER_HOME="$S" "$S/murmur-server.sh" gen-token --out "$d/token" >/dev/null 2>&1 ||
    die "the bundled murmur-server does not start with its libraries"
  if [ ! -f "$llm" ] || [ ! -d "$lib/parakeet-tdt-0.6b-v2-int8" ]; then
    echo "warning: no cleanup or parakeet model in this run; libmurmur_llm.so is not load-tested" >&2
    return 0
  fi
  port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  # The bundle's config with this run's paths; the [library] section is
  # left out (it would hash every model file).
  sed -e "s|^bind = .*|bind = [\"127.0.0.1:$port\"]|" \
      -e "s|^token_file = .*|token_file = \"$d/token\"|" \
      -e "s|^state_dir = .*|state_dir = \"$d/state\"|" \
      -e "s|/var/lib/murmur-server/library|$lib|g" -e "s|/opt/murmur-server|$S|g" \
      -e '/^\[library\]/,/^$/d' "$HERE/server-config.toml" > "$d/config.toml"
  [ -f "$lib/whisper-tiny-lid/ggml-tiny-q5_1.bin" ] || sed -i '/^lid_path/d' "$d/config.toml"
  [ -d "$lib/zipformer-he-int8" ] || sed -i '/^\[model.languages.he\]/,/^$/d' "$d/config.toml"
  say "smoke test: murmur-server with the cleanup model"
  MURMUR_SERVER_HOME="$S" "$S/murmur-server.sh" --config "$d/config.toml" > "$d/log" 2>&1 &
  pid=$!
  for ((i = 0; i < 300; i++)); do
    if grep -q '"ready"\| ready' "$d/log"; then ok=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  if [ "$ok" != 1 ] || ! grep -q 'llm warmed' "$d/log"; then
    tail -n 20 "$d/log" >&2
    die "the bundled murmur-server did not start with the cleanup model (log above)"
  fi
  echo "murmur-server started, cleanup plugin loaded and warmed"
}

# ---- murmur-server ------------------------------------------------------------
SERVER_NOTE="This bundle has no server build: use Docker (above), or ask for one."
if [ "$NO_SERVER" = 0 ]; then
  say "murmur-server"
  [ -n "$SHERPA_LIB" ] || die "--sherpa-lib DIR (or SHERPA_ONNX_LIB_DIR) with libsherpa-onnx-c-api.so + libonnxruntime.so is needed"
  for l in libsherpa-onnx-c-api.so libonnxruntime.so; do [ -e "$SHERPA_LIB/$l" ] || die "$SHERPA_LIB/$l not found"; done
  "$ROOT/tools/checks/no-tts-libs.sh" "$SHERPA_LIB/libsherpa-onnx-c-api.so" "$SHERPA_LIB/libonnxruntime.so" ||
    die "--sherpa-lib: use sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib/lib (D80: no espeak-ng)"
  if [ -z "$SERVER_TARGET" ]; then
    src="$OUT/.src/$SHA7"
    [ -d "$src" ] || g worktree add -q --detach "$src" "$COMMIT"
    avail=$(awk '/MemAvailable/ { print int($2 / 1024) }' /proc/meminfo)
    [ "$avail" -ge 4000 ] || die "only $avail MB of memory available; the build needs 4000 (or pass --server-target)"
    SERVER_TARGET="${CARGO_TARGET_DIR:-$HOME/.cache/murmur-bundle-target}/release"
    gcc_inc="$(gcc -print-file-name=include)"
    # No build path may end up in the binaries: the cargo registry and the
    # target dir sit under $HOME, and panic messages and llama.cpp's asserts
    # keep source paths. Map them away (the last matching rule wins).
    cargo_home="$(realpath -m "${CARGO_HOME:-$HOME/.cargo}")"
    remap="--remap-path-prefix=$HOME=~ --remap-path-prefix=$cargo_home=cargo --remap-path-prefix=$src=."
    cmap="-ffile-prefix-map=$HOME=~ -ffile-prefix-map=$cargo_home=cargo -ffile-prefix-map=$src=."
    (cd "$src" && export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/murmur-bundle-target}" &&
      export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$remap" CFLAGS="${CFLAGS:+$CFLAGS }$cmap" \
        CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$cmap" &&
      flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
        env SHERPA_ONNX_LIB_DIR="$SHERPA_LIB" cargo build --release --offline --locked -j 2 -p murmur-server \
          --no-default-features --features whisper,parakeet,tls,llm &&
      flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
        env BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I$gcc_inc}" \
        cargo build --release --offline --locked -j 2 -p murmur-llm)
  fi
  [ -x "$SERVER_TARGET/murmur-server" ] || die "$SERVER_TARGET/murmur-server not found"
  [ -f "$SERVER_TARGET/libmurmur_llm.so" ] || die "$SERVER_TARGET/libmurmur_llm.so not found"
  S="$BUNDLE/server"
  mkdir -p "$S/bin" "$S/lib"
  install -m 0755 "$SERVER_TARGET/murmur-server" "$S/bin/murmur-server"
  install -m 0644 "$SERVER_TARGET/libmurmur_llm.so" "$S/lib/libmurmur_llm.so"
  for l in libsherpa-onnx-c-api.so libonnxruntime.so; do install -m 0644 "$(readlink -f "$SHERPA_LIB/$l")" "$S/lib/$l"; done
  gomp="$(gcc -print-file-name=libgomp.so.1 2>/dev/null || true)"
  [ -e "$gomp" ] || die "libgomp.so.1 not found (the cleanup plugin needs it)"
  install -m 0644 "$(readlink -f "$gomp")" "$S/lib/libgomp.so.1"
  strip "$S/bin/murmur-server"
  strip --strip-unneeded "$S/lib/libmurmur_llm.so"
  install -m 0755 "$HERE/murmur-server.sh" "$S/murmur-server.sh"
  install -m 0644 "$HERE/server-config.toml" "$S/config.toml"
  g show "$COMMIT:crates/murmur-server/dist/murmur-server.service" > "$S/murmur-server.service"
  g show "$COMMIT:crates/murmur-server/dist/config.example.toml" > "$S/config.reference.toml"
  # The binary must be this commit's build (murmur_build_info::FULL is baked
  # in), and it must start with the bundled libraries.
  grep -aq "$VERSION+$SHA7" "$S/bin/murmur-server" ||
    die "$S/bin/murmur-server is not a clean build of $VERSION+$SHA7 (rebuild, or a --server-target from this commit)"
  # Nothing in the bundle may name this machine's home folder or user
  # (CI-built apps aren't built here; the copied libraries are checked too).
  user="$(id -un)" leaks=()
  for f in "$S"/bin/* "$S"/lib/*; do
    if grep -aqF -e "$HOME" -e "$user" "$f"; then leaks+=("${f#"$BUNDLE"/}"); fi
  done
  [ ${#leaks[@]} = 0 ] || die "these files contain the home path or user name: ${leaks[*]}"
  smoke_server "$S"
  SERVER_NOTE="The \`server/\` folder holds murmur-server $VERSION, built on $(lsb_release -ds 2>/dev/null || uname -sr). Docker (above) is simpler where it is available."
fi

# ---- murmur-server for Docker: source archive and image -----------------------
say "murmur-server source archive"
mkdir -p "$BUNDLE/docker"
# Tracked files of the commit only: nothing untracked or ignored can get in.
g archive --format=tar.gz --prefix="$SRC_DIR/" -o "$BUNDLE/docker/$SOURCE_TGZ" "$COMMIT" -- "${SRC_PATHS[@]}"
tar -tzf "$BUNDLE/docker/$SOURCE_TGZ" > "$WORK/src.list"
for f in deploy/docker/Dockerfile deploy/docker/entrypoint.sh Cargo.lock crates/murmur-server/Cargo.toml COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do
  grep -qx "$SRC_DIR/$f" "$WORK/src.list" || die "$SOURCE_TGZ has no $f"
done
echo "wrote docker/$SOURCE_TGZ ($(mb "$(stat -c %s "$BUNDLE/docker/$SOURCE_TGZ")"), $(wc -l < "$WORK/src.list") entries)"
if [ "$NO_IMAGE" = 0 ]; then
  say "murmur-server image (docker build of the source archive)"
  avail=$(awk '/MemAvailable/ { print int($2 / 1024) }' /proc/meminfo)
  [ "$avail" -ge 4000 ] || die "only $avail MB of memory available; the image build needs 4000 (or pass --no-image)"
  tar -xzf "$BUNDLE/docker/$SOURCE_TGZ" -C "$WORK"
  # The builder's own murmur-server:latest, if any, gets its tag back after.
  prev_latest=$(docker image inspect -f '{{.Id}}' murmur-server:latest 2>/dev/null || true)
  flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
    docker build -t "murmur-server:$VERSION" -t murmur-server:latest -f "$WORK/$SRC_DIR/deploy/docker/Dockerfile" \
      --build-arg MURMUR_GIT_SHA="$SHA7" --build-arg CARGO_BUILD_JOBS=2 "$WORK/$SRC_DIR"
  restore_latest() {
    docker rmi "murmur-server:$VERSION" >/dev/null 2>&1 || true
    if [ -n "$prev_latest" ]; then docker tag "$prev_latest" murmur-server:latest
    else docker rmi murmur-server:latest >/dev/null 2>&1 || true; fi
  }
  # This commit's build (murmur_build_info::FULL is baked in), and it starts
  # with the image's libraries.
  docker run --rm --entrypoint cat "murmur-server:$VERSION" /opt/murmur-server/bin/murmur-server |
    grep -acF "$VERSION+$SHA7" >/dev/null || { restore_latest; die "the image's murmur-server is not $VERSION+$SHA7"; }
  docker run --rm --entrypoint /opt/murmur-server/bin/murmur-server "murmur-server:$VERSION" gen-token --out /tmp/t >/dev/null ||
    { restore_latest; die "the image's murmur-server does not start"; }
  docker save -o "$BUNDLE/docker/$IMAGE_TAR" "murmur-server:$VERSION" murmur-server:latest
  restore_latest
  if grep -aqF -e "$HOME" -e "/$(id -un)/" "$BUNDLE/docker/$IMAGE_TAR"; then
    die "docker/$IMAGE_TAR contains this machine's home path or user name"
  fi
  echo "wrote docker/$IMAGE_TAR ($(mb "$(stat -c %s "$BUNDLE/docker/$IMAGE_TAR")"))"
fi

# ---- install note, checksums, zips -----------------------------------------
say "INSTALL.md"
GLIBC=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{ print $2 }')
DIAR=""; [ "$NOTETAKER" = 1 ] && DIAR=", meeting speakers"
VERSION="$VERSION" COMMIT="$SHA7" VARIANT="$VARIANT" WIN_EXE="$WIN_EXE" APK="$APK" \
  MODELS_ZIP="$MODELS_ZIP" CLEANUP_ZIP="$CLEANUP_ZIP" GLIBC="glibc $GLIBC" DIARIZER_LIST="$DIAR" \
  SERVER_NOTE="$SERVER_NOTE" NOTETAKER="$NOTETAKER" SOURCE_TGZ="$SOURCE_TGZ" IMAGE_TAR="$IMAGE_TAR" \
  IMAGE=$((1 - NO_IMAGE)) NOIMAGE="$NO_IMAGE" \
  python3 -I "$HERE/render_install.py" "$ROOT/docs/colleague-install.md" > "$BUNDLE/INSTALL.md"
grep -q '{{' "$BUNDLE/INSTALL.md" && die "INSTALL.md has an unfilled placeholder"
(cd "$BUNDLE" && find . -type f -printf '%P\n' | sort | xargs -d '\n' sha256sum) > "$WORK/sums.txt"
mv "$WORK/sums.txt" "$BUNDLE/SHA256SUMS.txt"

# Model zips (stored, not compressed), with a SHA256SUMS.txt inside from the
# pins (each file was checked above) and MODEL-LICENSES.txt: each model's
# licence and attribution and the full texts (D80).
zip_models() { # ZIP llm|speech
  local z="$OUT/$1" list="$WORK/$1.list"
  awk -F'\t' -v want="$2" '($2 == "llm") == (want == "llm")' "$WORK/models.tsv" > "$WORK/$1.tsv"
  [ -s "$WORK/$1.tsv" ] || return 0
  cut -f1 "$WORK/$1.tsv" | sort -u | xargs python3 -I "$HERE/model_notice.py" > "$MODELS/MODEL-LICENSES.txt"
  printf '%s\n' SHA256SUMS.txt MODEL-LICENSES.txt > "$list"
  while IFS=$'\t' read -r dir kind _ file _ sha _ _; do
    p=$(mpath "$dir" "$kind" "$file")
    echo "$p" >> "$list"
    [ "$kind" = diarizer ] || echo "library/$dir/model.toml" >> "$list"
    printf '%s  %s\n' "$sha" "$p"
  done < "$WORK/$1.tsv" > "$MODELS/SHA256SUMS.txt"
  rm -f "$z"
  (cd "$MODELS" && sort -u "$list" | zip -q -0 -@ "$z")
  rm -f "$MODELS/SHA256SUMS.txt" "$MODELS/MODEL-LICENSES.txt"
  unzip -l "$z" | grep -q ' MODEL-LICENSES.txt$' || die "$1 has no MODEL-LICENSES.txt"
  echo "wrote $z ($(mb "$(stat -c %s "$z")"))"
}
if [ "$NO_ZIP" = 0 ]; then
  say "zips"
  rm -f "$OUT/$NAME.zip"
  (cd "$OUT" && zip -q -r "$NAME.zip" "$NAME")
  echo "wrote $OUT/$NAME.zip ($(mb "$(stat -c %s "$OUT/$NAME.zip")"))"
  zip_models "$MODELS_ZIP" speech
  zip_models "$CLEANUP_ZIP" llm
  zips=("$NAME.zip")
  for z in "$MODELS_ZIP" "$CLEANUP_ZIP"; do [ ! -f "$OUT/$z" ] || zips+=("$z"); done
  (cd "$OUT" && sha256sum "${zips[@]}" > "$NAME-SHA256SUMS.txt")
fi
say "done: $BUNDLE"
