# hadolint global ignore=DL3006,DL3008
# (DL3006: the FROM lines name ARG-pinned images and stages; DL3008: apt
# packages follow the pinned base image.)
# flow-server container images (docs/container.md). Build context: repo root.
#
#   CPU, compiled in Docker (needs crates.io + GitHub for the sherpa-onnx libs):
#     docker build -f deploy/docker/Dockerfile -t flow-server .
#   CPU, from binaries built on the host (no network; tools/container/stage-prebuilt.sh first):
#     docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt -t flow-server:local .
#   NVIDIA CUDA variant (LLM cleanup on the GPU):
#     docker build -f deploy/docker/Dockerfile --target cuda -t flow-server:local-cuda .
#
# Needs BuildKit (Docker 23+ default; RUN --mount, COPY --chmod). No
# `# syntax=` line: the built-in frontend works without pulling one, so
# offline builds need only the base images.
# No models are baked in: mount a models folder at /models (read-only; one
# folder per model tagged by model.toml, e.g. the colleague bundle's
# library/). The entrypoint derives the config from it and keeps the admin
# key in the /data volume (D77). The Helm chart sets FLOW_CONFIG to its own
# config instead. The
# default target (last stage) is the CPU image.

ARG SOURCE=source
ARG RUST_IMAGE=rust:1.95-bookworm
ARG RUNTIME_IMAGE=ubuntu:24.04
ARG CUDA_DEVEL_IMAGE=nvidia/cuda:12.8.1-devel-ubuntu24.04
ARG CUDA_RUNTIME_IMAGE=nvidia/cuda:12.8.1-base-ubuntu24.04

# ---- artifacts-source: flow-server + libflow_llm.so built here ---------------
FROM ${RUST_IMAGE} AS artifacts-source-build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# The production feature set (docs/findings/2026-10-31-server-model-setup.md).
ARG FEATURES=whisper,parakeet,tls,llm
# Baked into `flow-server --version`/health (no .git in the context).
ARG FLOW_GIT_SHA=unknown
ARG CARGO_BUILD_JOBS=2
# sherpa-onnx-sys downloads its shared-lib release unless SHERPA_ONNX_LIB_DIR
# is set; staged libs in deploy/docker/prebuilt/lib are used when present.
# bindgen's libclang misses the compiler's own headers on some images.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    if [ -e deploy/docker/prebuilt/lib/libsherpa-onnx-c-api.so ]; then \
        export SHERPA_ONNX_LIB_DIR=/src/deploy/docker/prebuilt/lib; fi; \
    BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include)"; export BINDGEN_EXTRA_CLANG_ARGS; \
    export FLOW_GIT_SHA FLOW_GIT_DIRTY=0; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p flow-server \
        --no-default-features --features "$FEATURES"; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p flow-llm; \
    mkdir -p /out/bin /out/lib; \
    cp target/release/flow-server /out/bin/; \
    cp -L target/release/libflow_llm.so /out/lib/; \
    for l in libsherpa-onnx-c-api.so libonnxruntime.so; do \
        cp -L "${SHERPA_ONNX_LIB_DIR:-target/release}/$l" /out/lib/; done; \
    cp -L "$(gcc -print-file-name=libgomp.so.1)" /out/lib/

FROM scratch AS artifacts-source
COPY --from=artifacts-source-build /out/ /out/

# ---- artifacts-prebuilt: whatever tools/container/stage-prebuilt.sh staged ----
FROM scratch AS artifacts-prebuilt
COPY deploy/docker/prebuilt/ /out/

FROM artifacts-${SOURCE} AS artifacts

# ---- CUDA llama.cpp plugin -----------------------------------------------------
# Only LLM cleanup uses the GPU: sherpa-onnx (Parakeet/zipformer) runs with
# the CPU provider and whisper (LID) stays on the CPU (docs/container.md).
FROM ${RUST_IMAGE} AS rust-toolchain

FROM ${CUDA_DEVEL_IMAGE} AS cuda-llm-source-build
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
ARG FLOW_GIT_SHA=unknown
ARG CARGO_BUILD_JOBS=2
# llama-cpp-sys-2 links cudart/cuBLAS statically; only the driver's
# libcuda.so.1 is needed at run time, which the NVIDIA container toolkit
# injects. The link uses the toolkit's stub.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    export CUDAARCHS FLOW_GIT_SHA FLOW_GIT_DIRTY=0; \
    BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include)"; export BINDGEN_EXTRA_CLANG_ARGS; \
    export LIBRARY_PATH=/usr/local/cuda/lib64/stubs; \
    cargo build --release --locked -j "$CARGO_BUILD_JOBS" -p flow-llm --features cuda; \
    mkdir -p /out/lib; \
    cp -L target/release/libflow_llm.so /out/lib/libflow_llm_cuda.so

FROM scratch AS cuda-llm-source
COPY --from=cuda-llm-source-build /out/ /out/

FROM scratch AS cuda-llm-prebuilt
COPY deploy/docker/prebuilt/lib/libflow_llm_cuda.so /out/lib/

FROM cuda-llm-${SOURCE} AS cuda-llm

# ---- runtime images --------------------------------------------------------------
# Same layout in both:
#   /opt/flow-server/bin/flow-server (+ /usr/local/bin/flow-server link)
#   /opt/flow-server/lib  sherpa-onnx, onnxruntime, libflow_llm*.so, libgomp
#   /opt/flow-server/{COPYRIGHT,NOTICE}  rights and third-party licences
#   /usr/local/bin/flow-server -> the entrypoint (finds the config for
#   `docker exec … flow-server keys …`)
# uid/gid 10001; /data (and the chart's state dir) are created owned by it
# so a fresh named volume inherits the ownership.
FROM ${CUDA_RUNTIME_IMAGE} AS cuda
COPY --from=artifacts /out/bin/flow-server /opt/flow-server/bin/flow-server
COPY --from=artifacts /out/lib/ /opt/flow-server/lib/
COPY --from=cuda-llm /out/lib/libflow_llm_cuda.so /opt/flow-server/lib/
COPY --chmod=0755 deploy/docker/entrypoint.sh /usr/local/bin/flow-entrypoint
COPY --chmod=0755 deploy/docker/healthcheck.sh /usr/local/bin/flow-healthcheck
COPY COPYRIGHT NOTICE /opt/flow-server/
RUN set -eu; \
    ln -s /usr/local/bin/flow-entrypoint /usr/local/bin/flow-server; \
    groupadd -g 10001 flow; \
    useradd -u 10001 -g 10001 -M -d /data -s /usr/sbin/nologin flow; \
    mkdir -p /data /var/lib/flow-server /models; \
    chown 10001:10001 /data /var/lib/flow-server; \
    chmod 0700 /data /var/lib/flow-server
# FLOW_LLM_LIB: the plugin used when the config has llm_path but no llm_lib.
# The CUDA plugin needs libcuda.so.1 from the NVIDIA runtime (--gpus / the
# compose `gpu` profile / the chart's gpu.enabled). If the GPU fails at load,
# the plugin falls back to the CPU (flow-llm backend::choose); without the
# NVIDIA runtime set FLOW_LLM_LIB=/opt/flow-server/lib/libflow_llm.so.
ENV LD_LIBRARY_PATH=/opt/flow-server/lib \
    FLOW_LLM_LIB=/opt/flow-server/lib/libflow_llm_cuda.so \
    NVIDIA_VISIBLE_DEVICES=all \
    NVIDIA_DRIVER_CAPABILITIES=compute,utility
# Zero-config start (D77, entrypoint.sh): models from /models, key in /data.
ENV FLOW_DATA=/data \
    FLOW_MODELS_DIR=/models
USER 10001:10001
WORKDIR /data
EXPOSE 7313
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["/usr/local/bin/flow-healthcheck"]
ENTRYPOINT ["/usr/local/bin/flow-entrypoint"]
CMD ["serve"]

FROM ${RUNTIME_IMAGE} AS runtime
COPY --from=artifacts /out/bin/flow-server /opt/flow-server/bin/flow-server
COPY --from=artifacts /out/lib/ /opt/flow-server/lib/
COPY --chmod=0755 deploy/docker/entrypoint.sh /usr/local/bin/flow-entrypoint
COPY --chmod=0755 deploy/docker/healthcheck.sh /usr/local/bin/flow-healthcheck
COPY COPYRIGHT NOTICE /opt/flow-server/
RUN set -eu; \
    ln -s /usr/local/bin/flow-entrypoint /usr/local/bin/flow-server; \
    groupadd -g 10001 flow; \
    useradd -u 10001 -g 10001 -M -d /data -s /usr/sbin/nologin flow; \
    mkdir -p /data /var/lib/flow-server /models; \
    chown 10001:10001 /data /var/lib/flow-server; \
    chmod 0700 /data /var/lib/flow-server
ENV LD_LIBRARY_PATH=/opt/flow-server/lib \
    FLOW_LLM_LIB=/opt/flow-server/lib/libflow_llm.so
# Zero-config start (D77, entrypoint.sh): models from /models, key in /data.
ENV FLOW_DATA=/data \
    FLOW_MODELS_DIR=/models
USER 10001:10001
WORKDIR /data
EXPOSE 7313
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["/usr/local/bin/flow-healthcheck"]
ENTRYPOINT ["/usr/local/bin/flow-entrypoint"]
CMD ["serve"]
