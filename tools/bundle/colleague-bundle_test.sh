#!/usr/bin/env bash
# Checks for tools/colleague-bundle.sh's helpers (repo-checks CI; no network):
# the model list matches the apps' catalogue, a catalogue-only speaker model
# is added, a wrong pin fails, and the install note renders both variants.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
H="$ROOT/tools/bundle"
LIST="$ROOT/docs/air-gapped-models.txt"
RS="$ROOT/apps/flow-windows/src/model_download.rs"
CA="$ROOT/apps/flow-windows/src/core_adapter.rs"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. This tree's list and catalogue agree, line for line.
python3 -I "$H/models_manifest.py" "$LIST" "$RS" "$CA" > "$T/m.tsv" || fail "manifest of this tree"
want=$(grep -cvE '^\s*(#|$)' "$LIST")
[ "$(grep -cv $'\tdiarizer\t' "$T/m.tsv")" = "$want" ] || fail "manifest rows != list lines ($want)"

# 2. A Diarization entry only in the catalogue is added (kind diarizer).
python3 -I - "$RS" "$T/diar.rs" <<'PY'
import sys
s = open(sys.argv[1], encoding="utf-8").read()
entry = '''    CatalogEntry {
        id: "test-diarizer",
        title: "Speaker test",
        engine: ModelEngine::Diarization,
        dest: "speaker-test",
        files: &[model_file!(whisper_base!(), "spk.onnx", 1_234, "%s")],
    },
''' % ("ab" * 32)
i = s.index("\n];", s.index("pub const CATALOG"))
open(sys.argv[2], "w", encoding="utf-8").write(s[:i + 1] + entry + s[i + 1:])
PY
python3 -I "$H/models_manifest.py" "$LIST" "$T/diar.rs" "$CA" > "$T/d.tsv" || fail "manifest with a diarizer"
grep -q $'^speaker-test\tdiarizer\tsherpa_diarization\tspk.onnx\t1234\t' "$T/d.tsv" || fail "diarizer row missing"

# 3. A list pin the catalogue does not have fails.
sed '0,/^parakeet-tdt-0.6b-v2-int8 /s/ 652184296 / 652184297 /' "$LIST" > "$T/bad.txt"
if python3 -I "$H/models_manifest.py" "$T/bad.txt" "$RS" "$CA" > /dev/null 2>&1; then fail "wrong size accepted"; fi

# 4. The install note renders: no placeholder left, notetaker blocks only
# with NOTETAKER=1, the Docker image's lines only when it is bundled.
render() { # NOTETAKER IMAGE
  NOTETAKER=$1 IMAGE=$2 NOIMAGE=$((1 - $2)) VERSION=9.9.9 COMMIT=abc1234 VARIANT=v WIN_EXE=w.exe \
    APK=a.apk MODELS_ZIP=m.zip CLEANUP_ZIP=c.zip GLIBC="glibc 2.39" DIARIZER_LIST="" SERVER_NOTE="note" \
    SOURCE_TGZ=flow-server-source-9.9.9.tar.gz IMAGE_TAR=flow-server-image-9.9.9.tar \
    python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md"
}
render 0 1 > "$T/i0.md"; render 1 1 > "$T/i1.md"; render 0 0 > "$T/n0.md"
for f in "$T/i0.md" "$T/i1.md" "$T/n0.md"; do
  ! grep -q '{{\|<!--' "$f" || fail "placeholder or marker left in $(basename "$f")"
  grep -q 'tar -xzf docker/flow-server-source-9.9.9.tar.gz' "$f" || fail "source build missing in $(basename "$f")"
done
! grep -qi 'notetaker' "$T/i0.md" || fail "notetaker text in the integration note"
grep -q 'Speaker separation' "$T/i1.md" || fail "notetaker block missing"
grep -q 'docker load -i docker/flow-server-image-9.9.9.tar' "$T/i0.md" || fail "docker load missing"
! grep -q 'flow-server-image\|docker load' "$T/n0.md" || fail "image text without an image"
grep -q 'no prebuilt image' "$T/n0.md" || fail "no-image note missing"
if NOTETAKER=0 python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md" > /dev/null 2>&1; then
  fail "missing values accepted"
fi

# 5. The bundle's server config is valid TOML (the Rust test parses it as a Config).
python3 -I -c 'import sys, tomllib; tomllib.load(open(sys.argv[1], "rb"))' "$H/server-config.toml" || fail "server-config.toml"

# 6. COPYRIGHT and NOTICE (D80) travel with every bundle: the bundle root and
# its server source archive, the Docker build context and image, the Windows
# installer's resources and the APK's assets.
for f in COPYRIGHT NOTICE; do [ -s "$ROOT/$f" ] || fail "$f missing at the repo root"; done
grep -q 'All rights reserved' "$ROOT/COPYRIGHT" || fail "COPYRIGHT does not reserve rights"
grep -q '^--- BEGIN GENERATED: Rust crates' "$ROOT/NOTICE" || fail "NOTICE has no crate section"
B="$ROOT/tools/colleague-bundle.sh"
grep -q '^SRC_PATHS=(.* COPYRIGHT NOTICE ' "$B" || fail "source archive leaves COPYRIGHT/NOTICE out"
grep -q '^for f in COPYRIGHT NOTICE; do' "$B" || fail "bundle root leaves COPYRIGHT/NOTICE out"
grep -qx '!COPYRIGHT' "$ROOT/.dockerignore" && grep -qx '!NOTICE' "$ROOT/.dockerignore" || fail ".dockerignore drops COPYRIGHT/NOTICE"
[ "$(grep -c '^COPY COPYRIGHT NOTICE /opt/flow-server/$' "$ROOT/deploy/docker/Dockerfile")" = 2 ] || fail "an image leaves COPYRIGHT/NOTICE out"
python3 -I -c 'import json, sys
r = json.load(open(sys.argv[1]))["bundle"]["resources"]
assert r.get("../../COPYRIGHT") == "COPYRIGHT.txt" and r.get("../../NOTICE") == "NOTICE.txt", r' \
  "$ROOT/apps/flow-windows/tauri.conf.json" || fail "Windows installer resources lack COPYRIGHT/NOTICE"
grep -q 'sources.from(file("../../../COPYRIGHT"), file("../../../NOTICE"))' "$ROOT/apps/flow-android/app/build.gradle.kts" ||
  fail "APK assets lack COPYRIGHT/NOTICE"
echo "colleague-bundle helpers: ok"
