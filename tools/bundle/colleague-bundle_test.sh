#!/usr/bin/env bash
# Checks for tools/colleague-bundle.sh's helpers (repo-checks CI; no network):
# the model list matches the apps' catalogue, a catalogue-only speaker model
# is added, a wrong pin fails, the install note renders both variants, the
# notices travel with the bundle and each model zip's licence file is right.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
H="$ROOT/tools/bundle"
LIST="$ROOT/docs/air-gapped-models.txt"
RS="$ROOT/apps/murmur-windows/src/model_download.rs"
CA="$ROOT/apps/murmur-windows/src/core_adapter.rs"
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
    SOURCE_TGZ=murmur-server-source-9.9.9.tar.gz IMAGE_TAR=murmur-server-image-9.9.9.tar \
    python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md"
}
render 0 1 > "$T/i0.md"; render 1 1 > "$T/i1.md"; render 0 0 > "$T/n0.md"
for f in "$T/i0.md" "$T/i1.md" "$T/n0.md"; do
  ! grep -q '{{\|<!--' "$f" || fail "placeholder or marker left in $(basename "$f")"
  grep -q 'tar -xzf docker/murmur-server-source-9.9.9.tar.gz' "$f" || fail "source build missing in $(basename "$f")"
done
! grep -qi 'notetaker' "$T/i0.md" || fail "notetaker text in the integration note"
grep -q 'Speaker separation' "$T/i1.md" || fail "notetaker block missing"
grep -q 'docker load -i docker/murmur-server-image-9.9.9.tar' "$T/i0.md" || fail "docker load missing"
! grep -q 'murmur-server-image\|docker load' "$T/n0.md" || fail "image text without an image"
grep -q 'no prebuilt image' "$T/n0.md" || fail "no-image note missing"
if NOTETAKER=0 python3 -I "$H/render_install.py" "$ROOT/docs/colleague-install.md" > /dev/null 2>&1; then
  fail "missing values accepted"
fi

# 5. The bundle's server config is valid TOML (the Rust test parses it as a Config).
python3 -I -c 'import sys, tomllib; tomllib.load(open(sys.argv[1], "rb"))' "$H/server-config.toml" || fail "server-config.toml"

# 6. D80: COPYRIGHT, NOTICE and THIRD-PARTY-LICENSES are at the root and
# travel with every bundle. The built artefacts are checked where they are
# built: APK (android-build "APKs carry the notices"), installers
# (windows-build, tools/ci/check-installer-notices.sh), image (server-image
# "Image carries the notices"); here the bundle's own outputs.
for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do [ -s "$ROOT/$f" ] || fail "$f missing at the repo root"; done
grep -q 'All rights reserved' "$ROOT/COPYRIGHT" || fail "COPYRIGHT does not reserve rights"
grep -q '^--- BEGIN GENERATED: Rust crates' "$ROOT/NOTICE" || fail "NOTICE has no crate section"
! grep -qi 'espeak-ng \[FLAG' "$ROOT/NOTICE" || fail "NOTICE still lists espeak-ng as shipped"
B="$ROOT/tools/colleague-bundle.sh"
grep -q '^SRC_PATHS=(.* COPYRIGHT NOTICE THIRD-PARTY-LICENSES ' "$B" || fail "source archive leaves a notice out"
grep -q '^for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do' "$B" || fail "bundle root leaves a notice out"
python3 -I -c 'import json, sys
r = json.load(open(sys.argv[1]))["bundle"]["resources"]
for f in ("COPYRIGHT", "NOTICE", "THIRD-PARTY-LICENSES"):
    assert r.get("../../" + f) == f + ".txt", (f, r)' \
  "$ROOT/apps/murmur-windows/tauri.conf.json" || fail "Windows installer resources lack a notice"
# The source archive the bundle makes (git archive of SRC_PATHS) holds them:
# the archive of HEAD's tracked files, as the bundle builds it.
if git -C "$ROOT" cat-file -e HEAD:THIRD-PARTY-LICENSES 2>/dev/null; then
  git -C "$ROOT" archive --format=tar HEAD -- COPYRIGHT NOTICE THIRD-PARTY-LICENSES .dockerignore | tar -t > "$T/src.list"
  for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do grep -qx "$f" "$T/src.list" || fail "source archive has no $f"; done
fi
# The Docker build context (.dockerignore applied) holds them: with Docker,
# build a throwaway stage from the real context and list what it got.
if command -v docker > /dev/null && docker info > /dev/null 2>&1; then
  printf 'FROM scratch\nCOPY COPYRIGHT NOTICE THIRD-PARTY-LICENSES /n/\n' |
    DOCKER_BUILDKIT=1 docker build -q -f - -o "type=local,dest=$T/ctx" "$ROOT" > /dev/null ||
    fail "the Docker build context lacks a notice"
  for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do
    cmp -s "$T/ctx/n/$f" "$ROOT/$f" || fail "docker context: $f differs"
  done
  echo "docker context: COPYRIGHT NOTICE THIRD-PARTY-LICENSES ok"
fi
[ "$(grep -c '^COPY COPYRIGHT NOTICE THIRD-PARTY-LICENSES /opt/murmur-server/$' "$ROOT/deploy/docker/Dockerfile")" = 2 ] ||
  fail "an image stage leaves a notice out"

# third_party/licenses/ is what its SHA256SUMS says (tools/notice/fetch-texts.sh).
(cd "$ROOT/third_party/licenses" && sha256sum -c --quiet SHA256SUMS) || fail "third_party/licenses/SHA256SUMS"
! grep -q ' SHA256SUMS$' "$ROOT/third_party/licenses/SHA256SUMS" || fail "SHA256SUMS lists itself"

# 7. Model zips' MODEL-LICENSES.txt: the catalogue and tools/notice/models.tsv
# agree; the notice of a speech set and of the cleanup set with Llama carries
# the texts and notices their licences ask for; an unknown model is
# "unconfirmed", not given a licence.
MN="$H/model_notice.py"
python3 -I "$MN" --check-catalog "$RS" || fail "catalogue licences != tools/notice/models.tsv"
cut -f1 "$T/m.tsv" | sort -u > "$T/folders"
while read -r f; do
  grep -q "^$f"$'\t' "$ROOT/tools/notice/models.tsv" || fail "model folder $f has no licence row"
  grep -qF "$(grep "^$f"$'\t' "$ROOT/tools/notice/models.tsv" | cut -f3 | cut -d' ' -f1)" "$ROOT/NOTICE" ||
    fail "NOTICE does not name $f's licence"
done < "$T/folders"
awk -F'\t' '$2 != "llm"' "$T/m.tsv" | cut -f1 | sort -u | xargs python3 -I "$MN" > "$T/speech.txt"
grep -q 'Licensed by NVIDIA Corporation under the NVIDIA Open Model License' "$T/speech.txt" || fail "speech zip: NVIDIA notice"
grep -q '^NVIDIA Open Model License Agreement' "$T/speech.txt" || fail "speech zip: NVIDIA licence text"
grep -q 'Attribution 4.0 International' "$T/speech.txt" || fail "speech zip: CC-BY-4.0 text"
grep -q 'zipformer-he-int8: CC-BY-4.0 · Yakov Kolani / ivrit.ai' "$T/speech.txt" || fail "speech zip: CC-BY credit"
grep -q 'Permission is hereby granted, free of charge' "$T/speech.txt" || fail "speech zip: MIT text"
grep -q 'Converted to int8 ONNX (krut42) from thewh1teagle/parakeet-tdt-0.6b-v3-he, fine-tuned from nvidia/parakeet-tdt-0.6b-v3' "$T/speech.txt" ||
  fail "speech zip: v3-he derivation"
[ "$(grep -c '^- ' "$T/speech.txt")" = "$(grep -c '^    Converted\|^    Int8' "$T/speech.txt")" ] || fail "speech zip: a model without its derivation"
grep -q 'NVIDIA (nvidia/parakeet-tdt-0.6b-v3), fine-tuned by' "$ROOT/NOTICE" || fail "NOTICE: v3-he attribution lacks NVIDIA"
python3 -I "$MN" qwen2.5-1.5b-instruct-q4_k_m llama-3.2-3b-instruct-q4_k_m test-diarizer > "$T/llm.txt"
grep -q 'Built with Llama' "$T/llm.txt" || fail "cleanup zip: Built with Llama"
grep -q 'LLAMA 3.2 COMMUNITY LICENSE AGREEMENT' "$T/llm.txt" || fail "cleanup zip: Llama licence text"
grep -q 'Acceptable Use Policy' "$T/llm.txt" || fail "cleanup zip: Llama AUP"
grep -q 'Apache License' "$T/llm.txt" || fail "cleanup zip: Apache-2.0 text"
grep -q 'test-diarizer: licence unconfirmed' "$T/llm.txt" || fail "unknown model given a licence"
python3 -I "$MN" qwen2.5-1.5b-instruct-q4_k_m > "$T/qwen.txt"
! grep -q 'Llama' "$T/qwen.txt" || fail "Llama text without Llama"
# A catalogue licence that drifts from the table fails.
sed 's/license: Some("Apache-2.0 · Qwen (Qwen\/Qwen2.5-1.5B/license: Some("MIT · Qwen (Qwen\/Qwen2.5-1.5B/' "$RS" > "$T/drift.rs"
if python3 -I "$MN" --check-catalog "$T/drift.rs" 2> /dev/null; then fail "catalogue drift accepted"; fi
# The zip itself, as zip_models builds it: MODEL-LICENSES.txt is inside.
mkdir -p "$T/z" && cp "$T/llm.txt" "$T/z/MODEL-LICENSES.txt" && echo x > "$T/z/SHA256SUMS.txt"
(cd "$T/z" && printf '%s\n' SHA256SUMS.txt MODEL-LICENSES.txt | zip -q -0 -@ "$T/c.zip")
unzip -l "$T/c.zip" | grep -q ' MODEL-LICENSES.txt$' || fail "zip lacks MODEL-LICENSES.txt"
grep -q 'printf .%s\\n. SHA256SUMS.txt MODEL-LICENSES.txt > "$list"' "$B" || fail "zip_models leaves MODEL-LICENSES.txt out"
echo "colleague-bundle helpers: ok"
