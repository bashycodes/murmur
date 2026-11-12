# hadolint global ignore=DL3006,DL3008
# (DL3006: the FROM lines name ARG-pinned images and stages; DL3008: apt
# packages follow the pinned base image.)
# murmur-server container images (docs/container.md). Build context: repo root.
#
#   CPU, compiled in Docker (needs crates.io + GitHub for the sherpa-onnx libs):
#     docker build -f deploy/docker/Dockerfile -t murmur-server .
#   CPU, from binaries built on the host (no network; tools/container/stage-prebuilt.sh first):
#     docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt -t murmur-server:local .
#   NVIDIA CUDA variant (AI cleanup and whisper speech models on the GPU, D84):
#     docker build -f deploy/docker/Dockerfile --target cuda -t murmur-server:local-cuda .
#
# Needs BuildKit (Docker 23+ default; RUN --mount, COPY --chmod). No
# `# syntax=` line: the built-in frontend works without pulling one, so
# offline builds need only the base images.
# No models are baked in: mount a models folder at /models (read-only; one
# folder per model tagged by model.toml, e.g. the colleague bundle's
# library/). The entrypoint derives the config from it and keeps the admin
# key in the /data volume (D77). The Helm chart sets MURMUR_CONFIG to its own
# config instead. The
# default target (last stage) is the CPU image.

ARG SOURCE=source
ARG RUST_IMAGE=rust:1.95-bookworm
ARG RUNTIME_IMAGE=ubuntu:24.04
ARG CUDA_DEVEL_IMAGE=nvidia/cuda:12.8.1-devel-ubuntu24.04
ARG CUDA_RUNTIME_IMAGE=nvidia/cuda:12.8.1-base-ubuntu24.04

# ---- artifacts-source: murmur-server + libmurmur_llm.so built here ---------------
FROM ${RUST_IMAGE} AS artifacts-source-build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# The production feature set (docs/findings/2026-10-31-server-model-setup.md).
ARG FEATURES=whisper,whisper-plugin,parakeet,tls,llm
# Baked into `murmur-server --version`/health (no .git in the context).
ARG MURMUR_GIT_SHA=unknown
ARG CARGO_BUILD_JOBS=2
# sherpa-onnx: staged libs in deploy/docker/prebuilt/lib when present, else
# upstream's no-TTS shared-lib release, pinned (D80: the default release links
# espeak-ng, GPL-3.0, for text-to-speech, which murmur-server never uses).
# bindgen's libclang misses the compiler's own headers on some images.
ARG SHERPA_VERSION=1.13.8
ARG SHERPA_NO_TTS_SHA256=bf2d998c8b07012cd5098f3b92673bc1333fd9b927767d7cb664be8190d8bc0b
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    if [ -e deploy/docker/prebuilt/lib/libsherpa-onnx-c-api.so ]; then \
        export SHERPA_ONNX_LIB_DIR=/src/deploy/docker/prebuilt/lib; \
    else \
        n="sherpa-onnx-v$SHERPA_VERSION-linux-x64-shared-no-tts-lib"; \
        curl -fsSL -o /tmp/sherpa.tar.bz2 "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$SHERPA_VERSION/$n.tar.bz2"; \
        echo "$SHERPA_NO_TTS_SHA256  /tmp/sherpa.tar.bz2" | sha256sum -c -; \
        tar -xjf /tmp/sherpa.tar.bz2 -C /opt; rm /tmp/sherpa.tar.bz2; \
        export SHERPA_ONNX_LIB_DIR="/opt/$n/lib"; fi; \
    BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include)"; export BINDGEN_EXTRA_CLANG_ARGS; \
    export MURMUR_GIT_SHA MURMUR_GIT_DIRTY=0; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p murmur-server \
        --no-default-features --features "$FEATURES"; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p murmur-llm; \
    mkdir -p /out/bin /out/lib; \
    cp target/release/murmur-server /out/bin/; \
    cp -L target/release/libmurmur_llm.so /out/lib/; \
    for l in libsherpa-onnx-c-api.so libonnxruntime.so; do \
        cp -L "$SHERPA_ONNX_LIB_DIR/$l" /out/lib/; done; \
    cp -L "$(gcc -print-file-name=libgomp.so.1)" /out/lib/

FROM scratch AS artifacts-source
COPY --from=artifacts-source-build /out/ /out/

# ---- artifacts-prebuilt: whatever tools/container/stage-prebuilt.sh staged ----
FROM scratch AS artifacts-prebuilt
COPY deploy/docker/prebuilt/ /out/

FROM artifacts-${SOURCE} AS artifacts

# ---- CUDA plugins: llama.cpp (cleanup) and whisper.cpp (speech) ----------------
# On the GPU: AI cleanup and every whisper speech model (the stock or ivrit.ai
# turbo as the main engine or the Hebrew route, D84). sherpa-onnx
# (Parakeet/zipformer) runs with the CPU provider and the whisper language-ID
# model stays on the built-in CPU whisper (docs/container.md §7).
FROM ${RUST_IMAGE} AS rust-toolchain

FROM ${CUDA_DEVEL_IMAGE} AS cuda-plugins-source-build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust-toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust-toolchain /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY . .
# GPU architectures to compile kernels for (CMake's CUDAARCHS); fewer = faster
# build. 75 Turing, 80/86 Ampere, 89 Ada, 90 Hopper, 120 Blackwell.
ARG CUDAARCHS="75;80;86;89;90;120"
ARG MURMUR_GIT_SHA=unknown
ARG CARGO_BUILD_JOBS=2
# llama-cpp-sys-2 links cudart/cuBLAS statically. whisper-rs-sys links them
# dynamically (libcudart, libcublas, libcublasLt .so.12), so those three are
# copied next to the plugins. Either way the driver's libcuda.so.1 comes from
# the NVIDIA container toolkit at run time; the link uses the toolkit's stub.
# The whisper build gets the architectures as CMAKE_CUDA_ARCHITECTURES
# (whisper-rs-sys passes CMAKE_*/GGML_* variables to CMake; ggml ignores
# CUDAARCHS) and GGML_NATIVE=OFF, so its CPU fallback runs on any x86-64 CPU
# with AVX2, not only the build machine's.
# CUDAFLAGS (CMake's initial CMAKE_CUDA_FLAGS): nvcc's host objects must be
# position-independent to link into the plugins (.so); the cmake crate adds
# -fPIC to the C/C++ flags only, and without it rust-lld refuses ggml-cuda's
# objects ("relocation R_X86_64_PC32 ... recompile with -fPIC").
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    export CUDAARCHS MURMUR_GIT_SHA MURMUR_GIT_DIRTY=0 CUDAFLAGS="-Xcompiler=-fPIC"; \
    BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include)"; export BINDGEN_EXTRA_CLANG_ARGS; \
    export LIBRARY_PATH=/usr/local/cuda/lib64/stubs; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p murmur-llm --features cuda; \
    CMAKE_CUDA_ARCHITECTURES="$CUDAARCHS" GGML_NATIVE=OFF \
        cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p murmur-whisper --features cuda; \
    mkdir -p /out/lib; \
    cp -L target/release/libmurmur_llm.so /out/lib/libmurmur_llm_cuda.so; \
    cp -L target/release/libmurmur_whisper.so /out/lib/libmurmur_whisper_cuda.so; \
    for l in libcudart.so.12 libcublas.so.12 libcublasLt.so.12; do \
        cp -L "/usr/local/cuda/lib64/$l" /out/lib/; done

FROM scratch AS cuda-plugins-source
COPY --from=cuda-plugins-source-build /out/ /out/

# tools/container/stage-prebuilt.sh --cuda-llm / --cuda-whisper / --cuda-lib.
FROM scratch AS cuda-plugins-prebuilt
COPY deploy/docker/prebuilt/cuda/ /out/lib/

FROM cuda-plugins-${SOURCE} AS cuda-plugins

# ---- runtime images --------------------------------------------------------------
# Same layout in both:
#   /opt/murmur-server/bin/murmur-server (+ /usr/local/bin/murmur-server link)
#   /opt/murmur-server/lib  sherpa-onnx, onnxruntime, libmurmur_llm*.so, libgomp;
#                           CUDA image: + libmurmur_whisper_cuda.so, cudart, cuBLAS
#   /opt/murmur-server/{COPYRIGHT,NOTICE,THIRD-PARTY-LICENSES}  rights, third-party
#   notices and their full licence texts (D80)
#   /usr/local/bin/murmur-server -> the entrypoint (finds the config for
#   `docker exec … murmur-server keys …`)
# uid/gid 10001; /data (and the chart's state dir) are created owned by it
# so a fresh named volume inherits the ownership.
FROM ${CUDA_RUNTIME_IMAGE} AS cuda
COPY --from=artifacts /out/bin/murmur-server /opt/murmur-server/bin/murmur-server
COPY --from=artifacts /out/lib/ /opt/murmur-server/lib/
COPY --from=cuda-plugins /out/lib/ /opt/murmur-server/lib/
COPY --chmod=0755 deploy/docker/entrypoint.sh /usr/local/bin/murmur-entrypoint
COPY --chmod=0755 deploy/docker/healthcheck.sh /usr/local/bin/murmur-healthcheck
COPY COPYRIGHT NOTICE THIRD-PARTY-LICENSES /opt/murmur-server/
# D80: no espeak-ng/piper (TTS, GPL-3.0) code in the shipped libraries.
RUN set -eu; \
    if grep -a -l -E 'espeak-ng|espeak_|piper_phonemize|phontab' /opt/murmur-server/lib/*.so*; then \
        echo "a library above carries sherpa-onnx's TTS code (espeak-ng); stage the no-TTS build" >&2; exit 1; fi; \
    ln -s /usr/local/bin/murmur-entrypoint /usr/local/bin/murmur-server; \
    groupadd -g 10001 murmur; \
    useradd -u 10001 -g 10001 -M -d /data -s /usr/sbin/nologin murmur; \
    mkdir -p /data /var/lib/murmur-server /models; \
    chown 10001:10001 /data /var/lib/murmur-server; \
    chmod 0700 /data /var/lib/murmur-server
# MURMUR_LLM_LIB: the plugin used when the config has llm_path but no llm_lib.
# The CUDA plugin needs libcuda.so.1 from the NVIDIA runtime (--gpus / the
# compose `gpu` profile / the chart's gpu.enabled). If the GPU fails at load,
# the plugin falls back to the CPU (murmur-llm backend::choose); without the
# NVIDIA runtime set MURMUR_LLM_LIB=/opt/murmur-server/lib/libmurmur_llm.so.
# MURMUR_WHISPER_LIB (D84): whisper speech models load through the CUDA
# whisper plugin. Without libcuda.so.1 it does not load and the server uses
# its built-in CPU whisper, with a warning in the log; with the driver but no
# GPU the plugin runs on the CPU. MURMUR_WHISPER_LIB= (empty) skips it.
ENV LD_LIBRARY_PATH=/opt/murmur-server/lib \
    MURMUR_LLM_LIB=/opt/murmur-server/lib/libmurmur_llm_cuda.so \
    MURMUR_WHISPER_LIB=/opt/murmur-server/lib/libmurmur_whisper_cuda.so \
    NVIDIA_VISIBLE_DEVICES=all \
    NVIDIA_DRIVER_CAPABILITIES=compute,utility
# Zero-config start (D77, entrypoint.sh): models from /models, key in /data.
ENV MURMUR_DATA=/data \
    MURMUR_MODELS_DIR=/models
USER 10001:10001
WORKDIR /data
EXPOSE 7313
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["/usr/local/bin/murmur-healthcheck"]
ENTRYPOINT ["/usr/local/bin/murmur-entrypoint"]
CMD ["serve"]

FROM ${RUNTIME_IMAGE} AS runtime
COPY --from=artifacts /out/bin/murmur-server /opt/murmur-server/bin/murmur-server
COPY --from=artifacts /out/lib/ /opt/murmur-server/lib/
COPY --chmod=0755 deploy/docker/entrypoint.sh /usr/local/bin/murmur-entrypoint
COPY --chmod=0755 deploy/docker/healthcheck.sh /usr/local/bin/murmur-healthcheck
COPY COPYRIGHT NOTICE THIRD-PARTY-LICENSES /opt/murmur-server/
# D80: no espeak-ng/piper (TTS, GPL-3.0) code in the shipped libraries.
RUN set -eu; \
    if grep -a -l -E 'espeak-ng|espeak_|piper_phonemize|phontab' /opt/murmur-server/lib/*.so*; then \
        echo "a library above carries sherpa-onnx's TTS code (espeak-ng); stage the no-TTS build" >&2; exit 1; fi; \
    ln -s /usr/local/bin/murmur-entrypoint /usr/local/bin/murmur-server; \
    groupadd -g 10001 murmur; \
    useradd -u 10001 -g 10001 -M -d /data -s /usr/sbin/nologin murmur; \
    mkdir -p /data /var/lib/murmur-server /models; \
    chown 10001:10001 /data /var/lib/murmur-server; \
    chmod 0700 /data /var/lib/murmur-server
ENV LD_LIBRARY_PATH=/opt/murmur-server/lib \
    MURMUR_LLM_LIB=/opt/murmur-server/lib/libmurmur_llm.so
# Zero-config start (D77, entrypoint.sh): models from /models, key in /data.
ENV MURMUR_DATA=/data \
    MURMUR_MODELS_DIR=/models
USER 10001:10001
WORKDIR /data
EXPOSE 7313
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["/usr/local/bin/murmur-healthcheck"]
ENTRYPOINT ["/usr/local/bin/murmur-entrypoint"]
CMD ["serve"]
