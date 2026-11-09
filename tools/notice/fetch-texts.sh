#!/usr/bin/env bash
# Refresh third_party/licenses/ (D80): the upstream licence and notice files of
# what we ship that is not a Rust crate (C/C++ libraries, the Android NDK's
# libc++, the installer, Android Gradle libraries) and of the models we
# download, plus the standard SPDX texts. The files are committed; this only
# runs when a pinned version below changes. tools/notice/crate-licences.py
# builds THIRD-PARTY-LICENSES and the model-zip notices from them.
#
#   tools/notice/fetch-texts.sh SHERPA_BUILD_DIR
#
# SHERPA_BUILD_DIR: a tools/build-sherpa-android.sh build dir
# (…/build-arm64-v8a-no-tts), whose _deps/ hold the exact sources sherpa-onnx
# 1.13.8 links in (kaldi-native-fbank, openfst, eigen, …).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
deps="${1:?usage: fetch-texts.sh SHERPA_BUILD_DIR}/_deps"
L="$ROOT/third_party/licenses"
[ -d "$deps/kaldi_native_fbank-src" ] || { echo "no sherpa-onnx _deps under $deps" >&2; exit 1; }
rm -rf "$L"; mkdir -p "$L"
get() { # DEST URL
  mkdir -p "$(dirname "$L/$1")"
  curl -fsSL --retry 3 -o "$L/$1" "$2"
}
take() { # DEST SRC
  mkdir -p "$(dirname "$L/$1")"
  cp "$2" "$L/$1"
}

# sherpa-onnx 1.13.8 and what it links in (the _deps of its own build).
get sherpa-onnx/LICENSE https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v1.13.8/LICENSE
take kaldi-native-fbank/LICENSE "$deps/kaldi_native_fbank-src/LICENSE"
take kaldi-decoder/LICENSE "$deps/kaldi_decoder-src/LICENSE"
take kaldifst/LICENSE "$deps/kaldifst-src/LICENSE"
take openfst/COPYING "$deps/openfst-src/COPYING"
take simple-sentencepiece/LICENSE "$deps/simple-sentencepiece-src/LICENSE"
take kissfft/COPYING "$deps/kissfft-src/COPYING"
take kissfft/BSD-3-Clause "$deps/kissfft-src/LICENSES/BSD-3-Clause"
take kissfft/Unlicense "$deps/kissfft-src/LICENSES/Unlicense"
for f in COPYING.README COPYING.MPL2 COPYING.BSD COPYING.APACHE COPYING.MINPACK; do
  take "eigen/$f" "$deps/eigen-src/$f"
done
take hclust-cpp/LICENSE "$deps/hclust_cpp-src/LICENSE"
take nlohmann-json/LICENSE.MIT "$deps/json-src/LICENSE.MIT"

# ONNX Runtime 1.28.2 (bundled with sherpa-onnx).
get onnxruntime/LICENSE https://raw.githubusercontent.com/microsoft/onnxruntime/v1.28.2/LICENSE
get onnxruntime/ThirdPartyNotices.txt https://raw.githubusercontent.com/microsoft/onnxruntime/v1.28.2/ThirdPartyNotices.txt

# LLVM libc++ (libc++_shared.so from NDK r27, LLVM 18).
get llvm-libcxx/LICENSE.TXT https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-18.1.8/libcxx/LICENSE.TXT

# GNU libgomp (server bundle, container image).
get libgomp/GPL-3.0.txt https://www.gnu.org/licenses/gpl-3.0.txt
get libgomp/GCC-exception-3.1.txt https://raw.githubusercontent.com/spdx/license-list-data/v3.25.0/text/GCC-exception-3.1.txt

# Windows installers: NSIS 3.13 stub and the nsis_tauri_utils plugin 0.5.3;
# WiX 3.14.1, whose UI extension puts its custom-action DLL (wixca, path
# validation) and dialog resources into the MSI.
get nsis/COPYING https://raw.githubusercontent.com/kichik/nsis/v313/COPYING
get wix/LICENSE.TXT https://raw.githubusercontent.com/wixtoolset/wix3/wix3141rtm/LICENSE.TXT
get nsis-tauri-utils/LICENSE_MIT https://raw.githubusercontent.com/tauri-apps/nsis-tauri-utils/nsis_tauri_utils-v0.5.3/LICENSE_MIT
get nsis-tauri-utils/LICENSE_APACHE-2.0 https://raw.githubusercontent.com/tauri-apps/nsis-tauri-utils/nsis_tauri_utils-v0.5.3/LICENSE_APACHE-2.0

# Android (Gradle) libraries with their own licence files; AndroidX, Kotlin,
# Firebase and datatransport use the standard Apache-2.0 text below.
get kotlinx-coroutines/LICENSE.txt https://raw.githubusercontent.com/Kotlin/kotlinx.coroutines/1.10.2/LICENSE.txt
get guava/COPYING https://raw.githubusercontent.com/google/guava/v31.0.1/COPYING
get checker-qual/LICENSE.txt https://raw.githubusercontent.com/typetools/checker-framework/checker-framework-3.12.0/checker-qual/LICENSE.txt
get reactive-streams/COPYING https://raw.githubusercontent.com/reactive-streams/reactive-streams-jvm/v1.0.3/COPYING

# Models (the colleague bundle's model zips carry these).
get models/Llama-3.2-Community-License.txt https://raw.githubusercontent.com/meta-llama/llama-models/8d29d93fa5700a60532e0061a02ffa89d0acd3fc/models/llama3_2/LICENSE
get models/Llama-3.2-Acceptable-Use-Policy.md https://raw.githubusercontent.com/meta-llama/llama-models/8d29d93fa5700a60532e0061a02ffa89d0acd3fc/models/llama3_2/USE_POLICY.md
curl -fsSL --retry 3 https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/ |
  python3 -I "$ROOT/tools/notice/html_text.py" 'NVIDIA Open Model License Agreement' 'Version Release Date:' \
  > "$L/models/NVIDIA-Open-Model-License.txt"

# Standard texts (SPDX license-list-data v3.25.0) for licences a component
# declares without shipping its own file.
for id in Apache-2.0 MIT MIT-0 BSD-2-Clause BSD-3-Clause ISC Zlib Unicode-3.0 MPL-2.0 Unlicense \
          BSL-1.0 CC0-1.0 0BSD CDLA-Permissive-2.0 CC-BY-4.0 OFL-1.1 MS-RL; do
  get "spdx/$id.txt" "https://raw.githubusercontent.com/spdx/license-list-data/v3.25.0/text/$id.txt"
done
# Into a file outside $L first: written in place, SHA256SUMS would list itself.
sums="$(mktemp)"
(cd "$L" && find . -type f ! -name SHA256SUMS | sort | sed 's|^\./||' | xargs sha256sum) > "$sums"
mv "$sums" "$L/SHA256SUMS"; chmod 644 "$L/SHA256SUMS"
echo "wrote $(find "$L" -type f | wc -l) files to third_party/licenses/"
