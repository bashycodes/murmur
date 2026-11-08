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

# 4. The install note renders: no placeholder left, notetaker blocks only with NOTETAKER=1.
render() {
  NOTETAKER=$1 VERSION=9.9.9 COMMIT=abc1234 VARIANT=v WIN_EXE=w.exe APK=a.apk MODELS_ZIP=m.zip \
    CLEANUP_ZIP=c.zip GLIBC="glibc 2.39" DIARIZER_LIST="" SERVER_NOTE="note" \
    python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md"
}
render 0 > "$T/i0.md"; render 1 > "$T/i1.md"
for f in "$T/i0.md" "$T/i1.md"; do
  ! grep -q '{{\|<!--' "$f" || fail "placeholder or marker left in $(basename "$f")"
done
! grep -qi 'notetaker' "$T/i0.md" || fail "notetaker text in the integration note"
grep -q 'Speaker separation' "$T/i1.md" || fail "notetaker block missing"
if NOTETAKER=0 python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md" > /dev/null 2>&1; then
  fail "missing values accepted"
fi

# 5. The bundle's server config is valid TOML (the Rust test parses it as a Config).
python3 -I -c 'import sys, tomllib; tomllib.load(open(sys.argv[1], "rb"))' "$H/server-config.toml" || fail "server-config.toml"
echo "colleague-bundle helpers: ok"
