#!/usr/bin/env bash
# Unit test of the container entrypoint's zero-config start (D77,
# deploy/docker/entrypoint.sh) without Docker: a stub flow-server binary, a
# temp /data and a model list whose URLs are local file:// copies.
# Seconds; needs sh, curl, sha256sum. Run by CI (repo-checks).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EP="$ROOT/deploy/docker/entrypoint.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

# Stub server: gen-token writes a fixed token, check-config passes unless
# STUB_NO_KEYS is set, serve prints its arguments.
mkdir -p "$T/bin" "$T/src"
cat >"$T/bin/flow-server" <<'EOF'
#!/bin/sh
case "$1" in
  gen-token) (umask 077; echo "stub-secret-0123" >"$3"); exit 0 ;;
  check-config) [ -z "${STUB_NO_KEYS:-}" ] || { echo "no keys"; exit 1; }; exit 0 ;;
  keys) echo "stub keys $*"; exit 0 ;;
esac
echo "stub serve $*"
EOF
chmod +x "$T/bin/flow-server"

# Model files and a manifest in the real list's format.
printf 'stt-model-bytes' >"$T/src/enc.onnx"
printf 'tokens' >"$T/src/tokens.txt"
printf 'lid' >"$T/src/lid.bin"
printf 'gguf' >"$T/src/llm.gguf"
line() { # folder kind engine file title
    local f="$T/src/$4"
    echo "$1 $2 $3 $4 $(stat -c %s "$f") $(sha256sum "$f" | cut -d' ' -f1) file://$f $5"
}
{
    echo "# test list"
    line parakeet-tdt-0.6b-v2-int8 stt parakeet enc.onnx "Parakeet test"
    line parakeet-tdt-0.6b-v2-int8 stt parakeet tokens.txt "Parakeet test"
    line whisper-tiny-lid lid whisper lid.bin "LID test"
    line qwen2.5-1.5b-instruct-q4_k_m llm gguf llm.gguf "Qwen test"
    echo "zipformer-he-int8 hebrew sherpa_online bad.onnx 3 0000000000000000000000000000000000000000000000000000000000000000 file://$T/src/tokens.txt Hebrew bad hash"
} >"$T/models.txt"

run() { # DATA [env...]: run the entrypoint, output in $T/out
    local data="$1"
    shift
    mkdir -p "$T/run"
    env FLOW_BIN="$T/bin/flow-server" FLOW_DATA="$data" FLOW_CONFIG="$data/config.toml" \
        FLOW_MODEL_LIST="$T/models.txt" FLOW_RUN_DIR="$T/run" FLOW_KEY_POLL_S=0 "$@" \
        sh "$EP" >"$T/out" 2>&1
}

# 1. English preset: download, model.toml, config, the key printed once.
D="$T/d1"
mkdir -p "$D"
run "$D" FLOW_MODELS=english || { cat "$T/out"; fail "english start"; }
[ "$(cat "$D/library/parakeet-tdt-0.6b-v2-int8/enc.onnx")" = stt-model-bytes ] || fail "model not downloaded"
grep -q 'catalogue_id = "parakeet-tdt-0.6b-v2-int8"' "$D/library/parakeet-tdt-0.6b-v2-int8/model.toml" || fail "model.toml"
grep -q "^asr_path = \"$D/library/parakeet-tdt-0.6b-v2-int8\"" "$D/config.toml" || fail "asr_path"
grep -q '^asr_engine = "parakeet"' "$D/config.toml" || fail "asr_engine"
grep -q "^token_file = \"$D/admin.token\"" "$D/config.toml" || fail "token_file"
! grep -q '^llm_path\|^lid_path\|^\[model.languages' "$D/config.toml" || fail "english config has extras"
[ "$(stat -c %a "$D/admin.token")" = 600 ] || fail "admin.token mode"
grep -q "stub-secret-0123" "$T/out" || fail "key not printed on first start"
grep -q "docker exec flow-server cat $D/admin.token" "$T/out" || fail "show-again hint"
grep -q "stub serve --config $T/run/flow-server.config.toml" "$T/out" || fail "serve with rewritten config"
grep -q '^bind = \["127.0.0.1:7313"' "$T/run/flow-server.config.toml" || fail "bind rewrite"
ok "english preset: download, config, key shown"

# 2. Second start: nothing downloaded, the key not printed again, the config kept.
echo "# edited" >>"$D/config.toml"
rm "$T/src/enc.onnx"   # a download attempt would now fail
run "$D" FLOW_MODELS=english || { cat "$T/out"; fail "second start"; }
grep -q stub-secret-0123 "$T/out" && fail "key printed again"
grep -q "downloading" "$T/out" && fail "downloaded again"
grep -q "^# edited" "$D/config.toml" || fail "config overwritten"
ok "second start: no download, key not shown, config kept"
printf 'stt-model-bytes' >"$T/src/enc.onnx"

# 3. Default-style list: LID, cleanup and the main model in the config.
D="$T/d2"
mkdir -p "$D"
run "$D" FLOW_MODELS="english,whisper-tiny-lid qwen2.5-1.5b-instruct-q4_k_m" || { cat "$T/out"; fail "list start"; }
grep -q "^lid_path = \"$D/library/whisper-tiny-lid/lid.bin\"" "$D/config.toml" || fail "lid_path"
grep -q "^llm_path = \"$D/library/qwen2.5-1.5b-instruct-q4_k_m/llm.gguf\"" "$D/config.toml" || fail "llm_path"
! grep -q catalogue_id "$D/library/whisper-tiny-lid/model.toml" || fail "lid has catalogue_id"
ok "model list: lid + llm in config"

# 4. A seeded (pre-copied) folder with the right files is used, not downloaded.
D="$T/d3"
mkdir -p "$D/library/parakeet-tdt-0.6b-v2-int8"
cp "$T/src/enc.onnx" "$T/src/tokens.txt" "$D/library/parakeet-tdt-0.6b-v2-int8/"
run "$D" FLOW_MODELS=english || { cat "$T/out"; fail "seeded start"; }
grep -q downloading "$T/out" && fail "seeded model downloaded"
ok "seeded library used"

# 5. Errors: unknown model, hash mismatch, no speech model, unwritable data.
D="$T/d4"
mkdir -p "$D"
run "$D" FLOW_MODELS=nope && fail "unknown model accepted"
grep -q 'unknown model "nope"' "$T/out" || fail "unknown model message"
run "$D" FLOW_MODELS=zipformer-he-int8 && fail "bad hash accepted"
grep -q "does not match its pinned SHA-256" "$T/out" || fail "hash message"
[ ! -e "$D/library/zipformer-he-int8/bad.onnx.part" ] || fail ".part left behind"
run "$D" FLOW_MODELS=whisper-tiny-lid && fail "config without stt accepted"
grep -q "no speech model" "$T/out" || fail "no stt message"
if [ "$(id -u)" != 0 ]; then
    mkdir -p "$T/ro" && chmod 0555 "$T/ro"
    run "$T/ro" FLOW_MODELS=english && fail "read-only data accepted"
    grep -q "is not writable" "$T/out" || fail "not writable message"
fi
ok "errors: unknown model, bad hash, no speech model, unwritable /data"

# 6. A config elsewhere (Helm): no setup; with no keys it waits.
mkdir -p "$T/etc"
echo 'bind = ["0.0.0.0:7313"]' >"$T/etc/config.toml"
D="$T/d5"
mkdir -p "$D"
env FLOW_BIN="$T/bin/flow-server" FLOW_DATA="$D" FLOW_CONFIG="$T/etc/config.toml" \
    FLOW_MODEL_LIST="$T/models.txt" FLOW_RUN_DIR="$T/run" FLOW_KEY_POLL_S=0.2 FLOW_MODELS=english \
    STUB_NO_KEYS=1 sh "$EP" >"$T/out" 2>&1 &
pid=$!
sleep 0.5
grep -q "waiting for keys.json" "$T/out" || { cat "$T/out"; fail "no wait for keys"; }
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
{ [ ! -e "$D/config.toml" ] && [ ! -d "$D/library" ]; } || fail "setup ran for an outside config"
ok "outside config: no setup, waits for keys"

# 7. Called as flow-server (docker exec): CLI with the config added.
mkdir -p "$T/as"
ln -s "$EP" "$T/as/flow-server"
out=$(env FLOW_BIN="$T/bin/flow-server" FLOW_DATA="$T/d1" FLOW_CONFIG="$T/d1/config.toml" \
    sh "$T/as/flow-server" keys add phone)
[ "$out" = "stub keys keys add phone --config $T/d1/config.toml" ] || fail "cli keys: $out"
ok "flow-server CLI name adds --config"
echo "entrypoint tests passed"
