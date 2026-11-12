#!/usr/bin/env bash
# Unit test of the container entrypoint (D77, deploy/docker/entrypoint.sh)
# without Docker: a stub murmur-server binary, a temp /data and a models folder
# in the library layout (one folder per model, tagged by model.toml).
# Seconds; needs only sh. Run by CI (repo-checks).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EP="$ROOT/deploy/docker/entrypoint.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }
SECRET="stub-secret-0123"

# Stub server: gen-token writes a fixed key (or fails with STUB_GEN_FAIL),
# check-config passes unless STUB_NO_KEYS is set, serve prints its arguments.
mkdir -p "$T/bin"
cat >"$T/bin/murmur-server" <<EOF
#!/bin/sh
case "\$1" in
  gen-token) [ -z "\${STUB_GEN_FAIL:-}" ] || exit 1; (umask 077; echo "$SECRET" >"\$3"); exit 0 ;;
  check-config) [ -z "\${STUB_NO_KEYS:-}" ] || { echo "no keys"; exit 1; }; exit 0 ;;
  keys) echo "stub keys \$*"; exit 0 ;;
esac
echo "stub serve \$*"
EOF
chmod +x "$T/bin/murmur-server"

# model DIR ID KIND ENGINE [FILE]: a tagged model folder, as the colleague
# bundle's library/ has them.
model() {
    mkdir -p "$1/$2"
    printf 'kind = "%s"\ntitle = "%s test"\nengine = "%s"\n' "$3" "$2" "$4" >"$1/$2/model.toml"
    if [ -n "${5:-}" ]; then printf 'w' >"$1/$2/$5"; else printf 'w' >"$1/$2/encoder.onnx"; fi
}
M="$T/models"
model "$M" another-stt stt parakeet
model "$M" parakeet-tdt-0.6b-v2-int8 stt parakeet
model "$M" whisper-tiny-lid lid whisper ggml-tiny-q5_1.bin
model "$M" qwen2.5-1.5b-instruct-q4_k_m llm gguf qwen2.5-1.5b-instruct-q4_k_m.gguf
model "$M" zipformer-he-int8 hebrew sherpa_online
mkdir -p "$M/untagged"

run() { # DATA [env...]: run the entrypoint, output in $T/out
    local data="$1"
    shift
    mkdir -p "$T/run"
    env -u MURMUR_CONFIG MURMUR_BIN="$T/bin/murmur-server" MURMUR_DATA="$data" MURMUR_MODELS_DIR="$M" \
        MURMUR_RUN_DIR="$T/run" MURMUR_KEY_POLL_S=0 "$@" \
        sh "$EP" >"$T/out" 2>&1
}
G="$T/run/murmur-server.generated.toml"

# 1. First start: config from the folder, the key created but never printed.
D="$T/d1"
mkdir -p "$D"
run "$D" || { cat "$T/out"; fail "first start"; }
grep -q "^asr_path = \"$M/parakeet-tdt-0.6b-v2-int8\"" "$G" || fail "preferred stt"
grep -q '^asr_engine = "parakeet"' "$G" || fail "asr_engine"
grep -q "^lid_path = \"$M/whisper-tiny-lid/ggml-tiny-q5_1.bin\"" "$G" || fail "lid file"
grep -q "^llm_path = \"$M/qwen2.5-1.5b-instruct-q4_k_m/qwen2.5-1.5b-instruct-q4_k_m.gguf\"" "$G" || fail "llm file"
grep -q '^asr_backend = "sherpa_online"' "$G" || fail "hebrew engine"
grep -q "^dir = \"$M\"" "$G" || fail "library dir"
grep -q "^token_file = \"$D/admin.token\"" "$G" || fail "token_file"
grep -q "^state_dir = \"$D\"" "$G" || fail "state_dir"
[ "$(stat -c %a "$D/admin.token")" = 600 ] || fail "admin.token mode"
! grep -q "$SECRET" "$T/out" || fail "key printed to the log"
grep -q "docker exec murmur-server cat $D/admin.token" "$T/out" || fail "show-key hint"
grep -q "hostname -I" "$T/out" || fail "LAN address hint"
grep -q "stub serve --config $T/run/murmur-server.config.toml" "$T/out" || fail "serve with rewritten config"
grep -q '^bind = \["127.0.0.1:7313"' "$T/run/murmur-server.config.toml" || fail "bind rewrite"
ok "first start: config from the models folder, key created, not logged"

# 2. Second start: the key kept, not printed, a one-line hint.
run "$D" || { cat "$T/out"; fail "second start"; }
[ "$(cat "$D/admin.token")" = "$SECRET" ] || fail "key replaced"
! grep -q "$SECRET" "$T/out" || fail "key printed on restart"
grep -q "admin key: docker exec" "$T/out" || fail "restart hint"
ok "second start: key kept, not logged"

# 3. Overrides: MURMUR_STT picks another folder, none drops a model.
run "$D" MURMUR_STT=another-stt MURMUR_LLM=none MURMUR_HEBREW=none || { cat "$T/out"; fail "overrides"; }
grep -q "^asr_path = \"$M/another-stt\"" "$G" || fail "MURMUR_STT"
! grep -q '^llm_path\|^\[model.languages' "$G" || fail "none not honoured"
run "$D" MURMUR_STT=qwen2.5-1.5b-instruct-q4_k_m && fail "wrong-kind MURMUR_STT accepted"
grep -q 'is not a "stt" model' "$T/out" || fail "wrong-kind message"
ok "MURMUR_STT / MURMUR_LLM=none / wrong kind"

# 3b. D83: the old FLOW_* name still works with a notice; the new one wins.
run "$D" FLOW_STT=another-stt || { cat "$T/out"; fail "old FLOW_STT"; }
grep -q "^asr_path = \"$M/another-stt\"" "$G" || fail "FLOW_STT not read"
grep -q 'notice: FLOW_STT is the old name of MURMUR_STT' "$T/out" || fail "no FLOW_ notice"
run "$D" FLOW_STT=another-stt MURMUR_STT=parakeet-tdt-0.6b-v2-int8 || { cat "$T/out"; fail "both names"; }
grep -q "^asr_path = \"$M/parakeet-tdt-0.6b-v2-int8\"" "$G" || fail "MURMUR_STT did not win"
ok "old FLOW_* names: read with a notice, the new name wins"

# 3c. D84: the two whisper turbo models (the GPU image's options): stock turbo
# as the main speech engine, ivrit.ai turbo as the Hebrew route.
model "$M" whisper-large-v3-turbo-q5_0 stt whisper ggml-large-v3-turbo-q5_0.bin
model "$M" ivrit-whisper-large-v3-turbo-q5_0 hebrew whisper ggml-ivrit-large-v3-turbo-q5_0.bin
run "$D" MURMUR_STT=whisper-large-v3-turbo-q5_0 MURMUR_HEBREW=ivrit-whisper-large-v3-turbo-q5_0 ||
    { cat "$T/out"; fail "turbo picks"; }
grep -q '^asr_engine = "whisper"' "$G" || fail "turbo asr_engine"
grep -q "^asr_path = \"$M/whisper-large-v3-turbo-q5_0/ggml-large-v3-turbo-q5_0.bin\"" "$G" ||
    fail "turbo asr_path"
sed -n '/^\[model.languages.he\]/,/^$/p' "$G" >"$T/he"
grep -q '^asr_backend = "whisper"' "$T/he" || fail "ivrit turbo engine"
grep -q "^asr_path = \"$M/ivrit-whisper-large-v3-turbo-q5_0/ggml-ivrit-large-v3-turbo-q5_0.bin\"" "$T/he" ||
    fail "ivrit turbo file"
grep -q "speech whisper-large-v3-turbo-q5_0, .*Hebrew ivrit-whisper-large-v3-turbo-q5_0" "$T/out" ||
    fail "turbo picks not logged"
run "$D" || { cat "$T/out"; fail "default picks with turbo present"; }
grep -q "^asr_path = \"$M/parakeet-tdt-0.6b-v2-int8\"" "$G" || fail "turbo displaced Parakeet"
grep -q "^asr_path = \"$M/zipformer-he-int8\"" "$G" || fail "turbo displaced the zipformer"
rm -r "$M/whisper-large-v3-turbo-q5_0" "$M/ivrit-whisper-large-v3-turbo-q5_0"
ok "turbo: stock as MURMUR_STT, ivrit.ai as MURMUR_HEBREW; defaults unchanged"

# 4. Own config in /data wins; the CLI uses it too.
cp "$G" "$D/config.toml"
echo "# edited" >>"$D/config.toml"
run "$D" || { cat "$T/out"; fail "own config"; }
grep -q "^# edited" "$T/run/murmur-server.config.toml" || fail "own config not used"
env -u MURMUR_CONFIG MURMUR_BIN="$T/bin/murmur-server" MURMUR_DATA="$D" MURMUR_MODELS_DIR="$M" \
    MURMUR_RUN_DIR="$T/run" sh "$EP" keys list >"$T/out" 2>&1 || fail "keys via entrypoint"
grep -q "stub keys keys list --config $D/config.toml" "$T/out" || fail "keys --config"
rm "$D/config.toml"
ok "own /data/config.toml used by serve and CLI"

# 5. Errors: no models folder, no speech model, gen-token failure, read-only data.
run "$D" MURMUR_MODELS_DIR="$T/none" && fail "missing models folder accepted"
grep -q "no models folder at $T/none" "$T/out" || fail "missing folder message"
mkdir -p "$T/empty/untagged"
run "$D" MURMUR_MODELS_DIR="$T/empty" && fail "no stt accepted"
grep -q "no speech model in $T/empty" "$T/out" || fail "no stt message"
mkdir -p "$T/d5"
run "$T/d5" STUB_GEN_FAIL=1 && fail "gen-token failure accepted"
grep -q "could not write the admin key" "$T/out" || fail "gen-token message"
mkdir -p "$T/ro" && chmod 500 "$T/ro"
if [ "$(id -u)" != 0 ]; then
    run "$T/ro" && fail "read-only data accepted"
    grep -q "is not writable by uid" "$T/out" || fail "read-only message"
fi
chmod 700 "$T/ro"
ok "errors: no folder, no speech model, gen-token, read-only /data"

# 6. MURMUR_CONFIG (Helm): used as is, no admin.token, waits for keys.
cp "$G" "$T/helm.toml"
mkdir -p "$T/d6"
env MURMUR_BIN="$T/bin/murmur-server" MURMUR_DATA="$T/d6" MURMUR_CONFIG="$T/helm.toml" \
    MURMUR_MODELS_DIR="$T/none" MURMUR_RUN_DIR="$T/run" STUB_NO_KEYS=1 MURMUR_KEY_POLL_S=0.1 \
    timeout 1 sh "$EP" >"$T/out" 2>&1 && fail "started without keys"
grep -q "waiting for keys.json" "$T/out" || fail "no wait message"
[ ! -e "$T/d6/admin.token" ] || fail "admin.token created under MURMUR_CONFIG"
ok "MURMUR_CONFIG: waits for keys, no admin.token"

# 7. Called as murmur-server (docker exec): CLI with the right config.
mkdir -p "$T/cli" && ln -s "$EP" "$T/cli/murmur-server"
env -u MURMUR_CONFIG MURMUR_BIN="$T/bin/murmur-server" MURMUR_DATA="$D" MURMUR_MODELS_DIR="$M" \
    MURMUR_RUN_DIR="$T/run" sh "$T/cli/murmur-server" keys add phone >"$T/out" 2>&1 || fail "cli name"
grep -q "stub keys keys add phone --config $G" "$T/out" || fail "cli config"
ok "murmur-server name: CLI with the generated config"

echo "all entrypoint tests passed"
