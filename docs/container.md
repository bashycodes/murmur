# murmur-server in a container: Advanced (Docker, Compose, Helm)

The quick start is in [`docs/server-setup.md`](server-setup.md): build the
image from the source, mount a models folder and one volume, one
`docker run`. This page has the settings and everything else about the
image. The server inside is the same as the host install
([`docs/server-advanced.md`](server-advanced.md)).

| file | what |
|---|---|
| `deploy/docker/Dockerfile` | CPU image (default target `runtime`) and NVIDIA CUDA image (`--target cuda`) |
| `deploy/docker/entrypoint.sh` | the config from the models folder, the admin key, bind rewrite, CLI commands; also `/usr/local/bin/murmur-server` |
| `deploy/docker/healthcheck.sh` | TCP liveness check (`murmur-healthcheck`) |
| `deploy/docker-compose.yml` | Compose: the quick start as a file, `gpu` profile |
| `deploy/helm/murmur-server/` | Helm chart |
| `.github/workflows/server-image.yml` | CI: builds the CPU image and checks that it starts (nothing is published) |
| `tools/container/entrypoint_test.sh` | entrypoint unit test (no Docker; CI repo-checks) |
| `tools/container/smoke.sh` | the quick start end to end: key, dictation, `keys add`, restart |
| `tools/container/stage-prebuilt.sh` | stages host-built binaries for an offline image build |

What the image contains: `murmur-server` built with the production features
(`whisper,parakeet,tls,llm`), the llama.cpp plugin `libmurmur_llm.so`, the
sherpa-onnx and onnxruntime shared libraries and `libgomp`. **No models**,
and nothing in it downloads any: you mount them. There is no published
image; build it from the source (§8).

## 1. Mounts: `/models` and `/data`

| mount | what |
|---|---|
| `/models` (read-only) | the models folder: `<id>/` per model with its `model.toml` (D40; `docs/server-advanced.md` §10). The server uses them, and the apps download them from it (the library). A bundle's unzipped `library/` folder, or the `air-gapped.md` download loop's folder, as they are |
| `/data` (volume) | `admin.token` (the first admin key, `token_file`, the key `owner` in `keys list`, mode 0600), `keys.json`, `key-usage.json`, `library-hashes.json`; optionally your own `config.toml` (§3) |

The server runs as uid/gid 10001. A Docker named volume gets that owner by
itself. A host folder for `/data` (`-v /srv/murmur:/data`) must be owned by
it: `sudo chown -R 10001:10001 /srv/murmur`; the entrypoint says so when it
cannot write. The models folder only needs to be readable by everyone.

Backup: the `/data` volume (`docker run --rm -v murmur_data:/data -v "$PWD":/b
ubuntu tar czf /b/murmur_data.tgz -C /data .`); the models are your folder.

## 2. Settings (environment variables)

| variable | default | |
|---|---|---|
| `MURMUR_STT`, `MURMUR_HEBREW`, `MURMUR_LID`, `MURMUR_LLM` | (picked) | a model folder id to use instead of the default pick, or `none` (§3) |
| `MURMUR_THREADS` | CPUs, at most 4 | `[model] threads` |
| `MURMUR_PORT` | `7313` | port inside the container |
| `MURMUR_CONTAINER_NAME` | `murmur-server` | the container name in the hints the log prints |
| `MURMUR_BIND_CONTAINER_IP` | `1` | see Binding |
| `MURMUR_INSECURE_BIND_ANY` | `0` | see Binding |
| `MURMUR_START_GRACE_S` | `600` | healthcheck grace while the models load |
| `MURMUR_CONFIG` | (unset) | a config file elsewhere, used as is (Helm): no generated config, no admin key; the entrypoint waits for the first `keys add` |

## 3. The config

Without `/data/config.toml` the entrypoint writes the config on every start
from the folders in `/models` and their `model.toml` (`kind`, `engine`):

| role | config | picked |
|---|---|---|
| speech (`kind = "stt"`, required) | `[model] asr_engine`, `asr_path` | `parakeet-tdt-0.6b-v2-int8`, else `parakeet-unified-en-0.6b-int8`, else the first `stt` folder by name |
| language detection (`lid`) | `lid_path` | `whisper-tiny-lid`, else the first `lid` |
| AI cleanup (`llm`) | `llm_path` | `qwen2.5-1.5b-instruct-q4_k_m`, else the first `llm` |
| Hebrew (`hebrew`) | `[model.languages.he]` | `zipformer-he-int8`, else the first `hebrew` |

Whisper and GGUF models are the `.bin` / `.gguf` file in their folder;
sherpa-onnx models (Parakeet, zipformer) the folder. `MURMUR_STT=<id>` (and
the others) overrides a pick; `none` leaves that role out. The log's first
line says which models it took. Adding a model is copying its folder into
your models folder: the apps see it within a minute, and the server uses it
after a restart if it is the pick.

Your own config (tuning, TLS, other languages): copy the generated one into
the volume and edit it; from then on it is used as is (restart to apply):

```sh
docker exec murmur-server cp /tmp/murmur-server.generated.toml /data/config.toml
docker exec -it murmur-server vi /data/config.toml   # or docker cp it out and back
docker restart murmur-server
```

Every key: `crates/murmur-server/dist/config.example.toml`. Keep
`token_file = "/data/admin.token"` and `[library] dir = "/models"`.

## 4. Keys

The first start writes `/data/admin.token`. It never goes to the log; read
it with `docker exec murmur-server cat /data/admin.token`. One key per
person or device: `docker exec murmur-server murmur-server keys add phone`
(`keys list`, `keys revoke <name>`); details in `docs/server-advanced.md`
§4.

## 5. Docker Compose

`deploy/docker-compose.yml` is the quick start as a file, with a restart
policy, a read-only root filesystem, no capabilities and an 8 GB memory
cap. From the source folder:

```sh
MURMUR_MODELS_PATH=/path/to/models docker compose -f deploy/docker-compose.yml up -d --build
docker exec murmur-server cat /data/admin.token
```

Settings from the environment or a `.env` file next to the compose file:
`MURMUR_MODELS_PATH` (required), `MURMUR_STT` / `MURMUR_LLM` / `MURMUR_HEBREW` /
`MURMUR_LID`, `MURMUR_PUBLISH_ADDR` (default `0.0.0.0`; set the host's LAN or
VPN address on a host with a public address), `MURMUR_HOST_PORT`.

## 6. TLS (`wss://`)

Off by default: the server accepts only loopback, LAN and VPN peers, and a
VPN already encrypts. For a shared LAN without a VPN: make the certificate
as in `docs/server-advanced.md` §5, put it in the volume
(`docker exec murmur-server mkdir -p /data/tls`, then `docker cp`), add the
`[tls]` block to your own config (§3) with `/data/tls/...` paths, restart,
and paste the pin into the apps.

## 7. GPU (NVIDIA)

Only AI cleanup uses the GPU. See "NVIDIA CUDA image" below, then
`docker compose -f deploy/docker-compose.yml --profile gpu up -d --build murmur-server-gpu`
(it needs the NVIDIA container toolkit). Both services use the
`murmur_data` volume: run one at a time.

## 8. Build the image yourself

```sh
# CPU image, compiled inside Docker. Needs network: crates.io, and GitHub
# for the sherpa-onnx shared libraries (unless staged, see below).
docker build -f deploy/docker/Dockerfile -t murmur-server .
```

The build compiles whisper.cpp and llama.cpp: about 10 minutes on a 4-core box and 3-4 GB of
RAM with the default `CARGO_BUILD_JOBS=2` (raise it with `--build-arg` on a
bigger machine). Cargo's registry and `target/` live in BuildKit cache
mounts, so a rebuild after a code change is incremental; `docker builder
prune` frees them. `--build-arg MURMUR_GIT_SHA=<sha>` puts the commit into
`murmur-server --version`.

### Offline build from host binaries

On a machine without network, or to reuse a host build: build on the host,
stage the files, and build the image with `SOURCE=prebuilt`. That image
build compiles nothing and downloads nothing beyond the base image
(`docker load` can provide it).

```sh
# sherpa-onnx 1.13.8 shared libs (libsherpa-onnx-c-api.so, libonnxruntime.so)
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/lib
tools/container/stage-prebuilt.sh --build     # or without --build after your own
                                              # cargo build --release (features
                                              # whisper,parakeet,tls,llm) + -p murmur-llm
docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt -t murmur-server .
```

The image is Ubuntu 24.04 (glibc 2.39): build the binaries on a host with
that glibc or older.

### NVIDIA CUDA image

```sh
docker build -f deploy/docker/Dockerfile --target cuda -t murmur-server:local-cuda \
    --build-arg CUDAARCHS="86" .      # your GPU's compute capability; default: 75-120
```

What runs on the GPU: **LLM cleanup only.** The image carries a CUDA build
of the llama.cpp plugin (`murmur-llm --features cuda`; cuBLAS linked
statically) as `libmurmur_llm_cuda.so`, selected through `MURMUR_LLM_LIB`. ASR
(Parakeet / zipformer through sherpa-onnx) uses sherpa-onnx's CPU provider
and whisper (language ID) stays on the CPU; both are fast enough on a CPU
(ASR ~0.03-0.1x real time) while the LLM is the slow step (1.4-2.9 s per
cleanup on a 4-core CPU, `docs/findings/2026-10-31-server-model-setup.md`).
The plugin picks the GPU and falls back to the CPU when the model does not
fit (murmur-llm `backend::choose`). It needs the driver's `libcuda.so.1`,
which the NVIDIA container toolkit injects (`--gpus all`, the Compose `gpu`
profile, the chart's `gpu.enabled`). To run the CUDA image without a GPU,
set `MURMUR_LLM_LIB=/opt/murmur-server/lib/libmurmur_llm.so` (the CPU plugin is
in the image too).

For an offline CUDA image, build the plugin on a CUDA host
(`cargo build --release -p murmur-llm --features cuda`) and stage it with
`tools/container/stage-prebuilt.sh --cuda-llm target/release/libmurmur_llm.so`.

## 9. Air-gapped hosts: move the image

```sh
# on the build machine
docker save murmur-server:local | gzip > murmur-server-local.tar.gz
# copy the file (scp, USB), then on the target host
gunzip -c murmur-server-local.tar.gz | docker load
```

Kubernetes nodes read images from their own runtime, not from Docker:

```sh
k3s ctr images import murmur-server-local.tar               # k3s (gunzip first)
sudo ctr -n k8s.io images import murmur-server-local.tar    # containerd
kind load image-archive murmur-server-local.tar             # kind
k3d image import murmur-server-local.tar -c <cluster>       # k3d
```

The chart uses `imagePullPolicy: Never`, so a missing image shows as
`ErrImageNeverPull` instead of a pull from a registry.

## 10. Binding

murmur-server refuses a wildcard (`0.0.0.0`) or public bind without `[tls]`
(it would put plain `ws://` audio and tokens on every network), and drops
peers outside loopback / LAN / VPN (`docs/server-advanced.md` §3, config.rs
`check_bind`). The image keeps those rules:

* **Default (`MURMUR_BIND_CONTAINER_IP=1`):** the entrypoint replaces the
  config's `bind` with `127.0.0.1` and the container's own address (a
  private address on Docker and Kubernetes networks), on port `MURMUR_PORT`
  (7313). Published ports and Services reach that address, the bind is
  not a wildcard, and the peer filter stays on. With Docker's default
  port publishing the server sees the real client address, so clients
  from the LAN or VPN connect and public ones are dropped. Who can reach
  the port at all is set by the publish address (`-p ADDR:7313:7313`,
  Compose `MURMUR_PUBLISH_ADDR`). The quick start publishes on `0.0.0.0`
  (IPv4) on purpose: Docker forwards IPv6 connections to an IPv4-only
  container through its userland proxy, so the server would see the
  proxy's private address instead of the client's and the peer filter
  could not drop a public IPv6 client.
* **TLS (`wss://`):** §6. The rewritten bind is not a wildcard either way.
* **`MURMUR_INSECURE_BIND_ANY=1`** (with `MURMUR_BIND_CONTAINER_IP=0` and a
  `0.0.0.0` bind) passes `--insecure-bind-any`: no wildcard check and **no
  peer filter**. The entrypoint and the server both warn at startup. Only
  for a LAN-only host where the default does not work (an unusual container
  network); publish the port on a LAN or VPN address, never on a public one.

## 11. Health checks

`/v1/health` needs a key, and the server's auth is unchanged, so the image
checks only that the port accepts a TCP connection (`murmur-healthcheck`,
used by the Docker `HEALTHCHECK` and the chart's liveness probe). It also
reports healthy while the entrypoint waits for the first key (Helm), and for
`MURMUR_START_GRACE_S` (600 s) after the server process starts, because the
models load before the listener binds (84 s for Parakeet + Hebrew + LID +
Qwen on a loaded 4-core box). The chart's readiness probe is a plain TCP
check, so the Service sends traffic only once the server listens.

## 12. Helm

```sh
helm install murmur deploy/helm/murmur-server -n murmur --create-namespace
# models: copy into the models PVC (murmur-murmur-server-models), e.g. through a
# temporary pod that mounts it, or use your own PVC: --set persistence.models.existingClaim=NAME
kubectl -n murmur exec deploy/murmur-murmur-server -- murmur-server keys add owner --admin
kubectl -n murmur logs -f deploy/murmur-murmur-server
```

The chart keeps its own layout instead of `/models` + `/data`: it sets
`MURMUR_CONFIG=/etc/murmur-server/config.toml` (its ConfigMap), so the
entrypoint writes no config and no admin key, and waits for the first key
(`keys add` above).

What the chart creates: a Deployment (one replica, `Recreate`: one writer
per state volume), PVCs for the models (mounted read-only) and the state
(both kept on `helm uninstall`), a ConfigMap with `config.toml`
(`values.config`; a change rolls the pod), a Service, and optionally an
Ingress.

| value | default | |
|---|---|---|
| `image.repository` / `image.tag` | `murmur-server` / `local` | locally loaded image, `pullPolicy: Never` |
| `gpu.enabled` | `false` | CUDA image (`gpu.image`, tag `local-cuda`) + `nvidia.com/gpu: 1`; `gpu.runtimeClassName` if needed |
| `bindPodIP` | `true` | bind 127.0.0.1 + the pod IP (Binding above) |
| `insecureBindAny` | `false` | `--insecure-bind-any`; refused together with `serverTls` |
| `serverTls.enabled` / `secretName` | off | `wss://` by murmur-server from a `kubernetes.io/tls` Secret; the chart adds `[tls]` |
| `service.type` | `ClusterIP` | `LoadBalancer` on a LAN: set `service.externalTrafficPolicy: Local` so the peer filter sees client addresses |
| `ingress.enabled` | `false` | Ingress for `/v1` (WebSocket `/v1/stream`, `/v1/health`, `/v1/models`), TLS at the controller |
| `persistence.models` / `.state` | 10Gi / 256Mi | `existingClaim`, `storageClass`, `size`, `accessModes` |
| `resources` | 1 CPU, 2-8 GiB | Parakeet ~1 GB RSS, + ~1.5 GB with Qwen2.5-1.5B |

Ingress notes: the hop from the controller to the pod is plain `ws://`
inside the cluster (the controller's pod address passes the peer filter).
Sessions last up to 20 minutes, so raise the controller's WebSocket
timeouts (the nginx annotations are in `values.yaml`). With
`serverTls.enabled` and an Ingress, the controller must speak TLS to the
backend (nginx: `nginx.ingress.kubernetes.io/backend-protocol: HTTPS`).

## 13. Smoke test

```sh
tools/container/smoke.sh --image murmur-server --wav /path/to/jfk.wav:en --models /path/to/models
```

Runs the quick start's `docker run` (container `murmur-smoke`, volume
`murmur-smoke-data`, host port 17313) with the models folder: the admin key
read with `docker exec` and checked to be absent from the log, one
dictation through the WebSocket protocol with it, `keys add` through
`docker exec` and a dictation with that key, then a restart (the same key
still works, still not logged). It removes the container and the volume at
the end.
