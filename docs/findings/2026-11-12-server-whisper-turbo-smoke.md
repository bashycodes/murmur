# murmur-server: whisper turbo smoke run (CPU) and CUDA image build (2026-11-12, D84)

Evidence for D84's acceptance line "stock turbo can be the main
`asr_engine` and passes a server smoke test". No CUDA GPU was reachable, so
this is the **CPU fallback** path: the server was told to use a GPU plugin
that does not exist. It covers the config, the library tag, the pin
(fingerprint) and the fallback warning. It does not cover CUDA speed.

## Setup

* Host: Intel Core i7-6700 (4 cores / 8 threads), no NVIDIA GPU, Linux.
* Server: `murmur-server 1.2.0+cf88a66.dirty` (branch
  `feat/server-whisper-cuda`), a debug build with the default features
  (`whisper`, `whisper-plugin`, `tls`). whisper.cpp is built at opt-level 3
  even in debug (root `Cargo.toml`), so decode speed matches a release build.
* Model: `ggml-large-v3-turbo-q5_0.bin` from the pinned URL
  (`ggerganov/whisper.cpp` @ `5359861c…58b1`). `sha256sum -c` against the
  catalogue pin `39422170…a7e2` passed, 574 041 195 B. It went into its own
  library folder `whisper-large-v3-turbo-q5_0/` with this `model.toml`:

  ```toml
  kind = "stt"
  title = "Whisper large-v3 turbo"
  catalogue_id = "whisper-large-v3-turbo-q5_0"
  engine = "whisper"
  ```

* Config (scratch dir, loopback only, a fresh random token file):

  ```toml
  bind = ["127.0.0.1:7399"]
  token_file = "<scratch>/token"
  state_dir = "<scratch>/state"
  workers = 1

  [model]
  asr_engine = "whisper"
  asr_path = "<library>/whisper-large-v3-turbo-q5_0/ggml-large-v3-turbo-q5_0.bin"
  whisper_lib = "/nonexistent/libmurmur_whisper_cuda.so"   # forces the CPU fallback

  [library]
  dir = "<library>"
  rescan_s = 0
  ```

* Audio: `jfk.wav`, the public 11 s whisper.cpp sample (16 kHz mono). It is
  the repo's existing test fixture, not owner audio.

## Run

```sh
murmur-server check-config --config config.toml   # config ok: ... workers=1 asr=Whisper
murmur-server serve --config config.toml
tools/probes/prototypes/server_e2e_2026_10_31.py --url ws://127.0.0.1:7399 \
    --token-file token --health --wav jfk.wav:en
```

## Results

Start log (the fallback warning, then ready, 6.0 s to load):

```text
WARN murmur_server::pool: speech model: whisper runs on the CPU, the GPU is not used: whisper GPU plugin not found (/nonexistent/libmurmur_whisper_cuda.so: ... cannot open shared object file: No such file or directory)
INFO murmur_server: ready version="1.2.0+cf88a66.dirty" workers=1 asr=whisper languages= load_ms=6007 listening=127.0.0.1:7399
```

`/v1/health` and `hello_ok`: `asr_backend = "whisper"`,
`asr_model = "ggml-large-v3-turbo-q5_0.bin"`.

The dictation:

| audio | status | asr_ms | commit → result | text |
|---|---|---|---|---|
| 11.0 s (7.2 s speech), `languages = ["en"]` | `RAW_TRANSCRIPT` | 13 155 | 13 182 ms | "And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country." (exact) |

`GET /v1/models`, after the first scan had hashed the file (about 25 s after
the start):

```text
whisper-large-v3-turbo-q5_0  catalogue_id=whisper-large-v3-turbo-q5_0  kind=stt
  ggml-large-v3-turbo-q5_0.bin  574041195 B  sha256 394221709cd5…
```

The served hash is the catalogue pin, so the apps accept the server's copy.

## What it shows

* Stock turbo works as the main engine through config, library tag and pin.
* A whisper plugin that does not load leaves the model on the built-in CPU
  whisper, and the log says why.
* On a CPU, turbo costs about 13 s per dictation at 4 threads. That is the
  number `docs/container.md` §7 quotes, and why Parakeet stays the default
  on a CPU.

## The CUDA image, built here without a GPU

`docker buildx build -f deploy/docker/Dockerfile --target cuda --build-arg
CUDAARCHS=86 --build-arg CARGO_BUILD_JOBS=2`, on an isolated buildx builder
(removed afterwards), test tag only.

* First try: **failed**. `murmur-llm --features cuda` did not link.
  rust-lld refused ggml-cuda's nvcc objects: "relocation R_X86_64_PC32
  cannot be used against symbol 'stderr'; recompile with -fPIC". The CUDA
  stage had never been built since the toolchain moved to rust-lld; this was
  not new with D84. The fix is `CUDAFLAGS="-Xcompiler=-fPIC"` in the stage
  (CMake starts `CMAKE_CUDA_FLAGS` from it).
* Second try: **green in 45 min** (2 709 s with cold cargo caches, 2 jobs,
  one architecture). Image: 1.32 GB content, 3.46 GB on disk.
* `/opt/murmur-server/lib`: `libmurmur_llm_cuda.so` 851 MB,
  `libcublasLt.so.12` 752 MB, `libcublas.so.12` 116 MB,
  `libmurmur_whisper_cuda.so` 55 MB, `libcudart.so.12` 0.7 MB. `ldd`:
  both plugins resolve every library except the driver's `libcuda.so.1`,
  as designed.
* Started without `--gpus`, with a whisper tiny model as `/models`:

  ```text
  WARN murmur_server::pool: speech model: whisper runs on the CPU, the GPU is not used: whisper GPU plugin not found (/opt/murmur-server/lib/libmurmur_whisper_cuda.so: libcuda.so.1: cannot open shared object file: No such file or directory)
  INFO murmur_server: ready version="1.2.0+d84test" workers=1 asr=whisper ...
  ```

Not covered: the CUDA plugin on a GPU (no NVIDIA GPU here), so GPU
latency is still the estimate in `docs/container.md` §7. ivrit.ai turbo on
the Hebrew route was not run either; only the entrypoint's stub test picks
it.
