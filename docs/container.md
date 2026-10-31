# flow-server in a container (Docker, Compose, Helm)

The same server as the host install (`docs/server-setup.md`), packaged as a
local container image. Files:

| file | what |
|---|---|
| `deploy/docker/Dockerfile` | CPU image (default target `runtime`) and NVIDIA CUDA image (`--target cuda`) |
| `deploy/docker/entrypoint.sh` | bind rewrite, waits for the first key, runs CLI commands |
| `deploy/docker/healthcheck.sh` | TCP liveness check (`flow-healthcheck`) |
| `deploy/docker/config.toml` | the default config for the container paths |
| `deploy/docker-compose.yml` | Compose: CPU service, `gpu` profile |
| `deploy/helm/flow-server/` | Helm chart |
| `tools/container/stage-prebuilt.sh` | stages host-built binaries for an offline image build |
| `tools/container/smoke.sh` | Compose smoke test: key bootstrap, a dictation, keys kept across `down`/`up` |

What the image contains: `flow-server` built with the production features
(`whisper,parakeet,tls,llm`), the llama.cpp plugin `libflow_llm.so`, the
sherpa-onnx and onnxruntime shared libraries, and `libgomp`. **No models**:
models, the model library, the keys and the config are all mounted.

| path in the container | contents | mount |
|---|---|---|
| `/models` | ASR model dirs, GGUF, `library/` (models served to the apps, D40) | read-only volume |
| `/var/lib/flow-server` | `keys.json`, `key-usage.json`, `library-hashes.json` | read-write volume |
| `/etc/flow-server/config.toml` | the config | read-only file |
| `/etc/flow-server/tls/` | `cert.pem` / `key.pem` (Compose) or `tls.crt` / `tls.key` (Helm) for `[tls]` | optional |
| `/tmp` | entrypoint state, the rewritten config | tmpfs |

The server runs as uid/gid 10001 on a read-only root filesystem with no
capabilities. A bind-mounted models folder must be readable by that uid
(world-readable is enough); a bind-mounted state folder must be owned by it
(`sudo chown -R 10001:10001 DIR`). Docker named volumes and the Helm
chart (`fsGroup`) handle that themselves.

## 1. Build the image

Images are built locally; nothing is published to a registry. From the
repo root:

```sh
# CPU image, compiled inside Docker. Needs network: crates.io, and GitHub
# for the sherpa-onnx shared libraries (unless staged, see below).
docker build -f deploy/docker/Dockerfile -t flow-server:local \
    --build-arg FLOW_GIT_SHA="$(git rev-parse --short=7 HEAD)" .
```

The build compiles whisper.cpp and llama.cpp: about 20 minutes and 3-4 GB of
RAM with the default `CARGO_BUILD_JOBS=2` (raise it with `--build-arg` on a
bigger machine).

### Offline build from host binaries

On a machine without network, or to reuse a host build: build on the host,
stage the files, and build the image with `SOURCE=prebuilt`. That image
build compiles nothing and downloads nothing (beyond the `ubuntu:24.04`
base image, which `docker load` can provide).

```sh
# sherpa-onnx 1.13.8 shared libs (libsherpa-onnx-c-api.so, libonnxruntime.so)
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/lib
tools/container/stage-prebuilt.sh --build     # or without --build after your own
                                              # cargo build --release (features
                                              # whisper,parakeet,tls,llm) + -p flow-llm
docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt -t flow-server:local .
```

The image is Ubuntu 24.04 (glibc 2.39): build the binaries on a host with
that glibc or older. Staged sherpa libs are also used by the in-Docker
build (then it does not download them).

### NVIDIA CUDA image

```sh
docker build -f deploy/docker/Dockerfile --target cuda -t flow-server:local-cuda \
    --build-arg CUDAARCHS="86" .      # your GPU's compute capability; default: 75-120
```

What runs on the GPU: **LLM cleanup only.** The image carries a CUDA build
of the llama.cpp plugin (`flow-llm --features cuda`; cuBLAS linked
statically) as `libflow_llm_cuda.so`, selected through `FLOW_LLM_LIB`. ASR
(Parakeet / zipformer through sherpa-onnx) uses sherpa-onnx's CPU provider
and whisper (language ID) stays on the CPU; both are fast enough on a CPU
(ASR ~0.03-0.1x real time) while the LLM is the slow step (1.4-2.9 s per
cleanup on a 4-core CPU, `docs/findings/2026-10-31-server-model-setup.md`).
The plugin picks the GPU and falls back to the CPU when the model does not
fit (flow-llm `backend::choose`). It needs the driver's `libcuda.so.1`,
which the NVIDIA container toolkit injects (`--gpus all`, the Compose `gpu`
profile, the chart's `gpu.enabled`). To run the CUDA image without a GPU,
set `FLOW_LLM_LIB=/opt/flow-server/lib/libflow_llm.so` (the CPU plugin is
in the image too).

For an offline CUDA image, build the plugin on a CUDA host
(`cargo build --release -p flow-llm --features cuda`) and stage it with
`tools/container/stage-prebuilt.sh --cuda-llm target/release/libflow_llm.so`.

## 2. Air-gapped hosts: move the image

```sh
# on the build machine
docker save flow-server:local | gzip > flow-server-local.tar.gz
# copy the file (scp, USB), then on the target host
gunzip -c flow-server-local.tar.gz | docker load
```

Kubernetes nodes read images from their own runtime, not from Docker:

```sh
k3s ctr images import flow-server-local.tar               # k3s (gunzip first)
sudo ctr -n k8s.io images import flow-server-local.tar    # containerd
kind load image-archive flow-server-local.tar             # kind
k3d image import flow-server-local.tar -c <cluster>       # k3d
```

The chart uses `imagePullPolicy: Never`, so a missing image shows as
`ErrImageNeverPull` instead of a pull from a registry.

## 3. Docker Compose

```sh
cd deploy
mkdir -p models && cp -r /path/to/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 models/
docker compose build                 # or: FLOW_BUILD_SOURCE=prebuilt docker compose build
docker compose up -d
docker compose exec flow-server flow-server keys add owner --admin
docker compose logs -f flow-server   # "keys found, starting", then "ready ... asr=parakeet"
```

The first start has no keys, so the server would refuse to start. The
entrypoint waits instead ("waiting for keys.json") until
`keys add … --admin` creates the first key, then starts the server. The
key's secret is printed once: paste it into the apps as the token. More
keys, one per device: `docker compose exec flow-server flow-server keys add
phone`, `keys list`, `keys revoke <name>` (D38; `docs/server-setup.md` §4).
The running server picks up key changes on the next connection. Keys live in
the `flow-state` volume and survive `docker compose down` and image
rebuilds (`down -v` deletes them).

Settings, from the environment or `deploy/.env`:

| variable | default | |
|---|---|---|
| `FLOW_MODELS_DIR` | `./models` | host models folder (read-only) |
| `FLOW_CONFIG_FILE` | `./docker/config.toml` | the config; copy it to change models, LLM, languages |
| `FLOW_PUBLISH_ADDR` | `127.0.0.1` | host address the port is published on: set the host's VPN or LAN address |
| `FLOW_HOST_PORT` | `7313` | host port |
| `FLOW_BIND_CONTAINER_IP` | `1` | see Binding |
| `FLOW_INSECURE_BIND_ANY` | `0` | see Binding |
| `FLOW_BUILD_SOURCE` | `source` | `prebuilt`: offline build from staged binaries |

GPU: `docker compose --profile gpu up -d flow-server-gpu` (instead of the
CPU service; it uses the same state volume, so the keys carry over). It
reserves one NVIDIA GPU and needs the NVIDIA container toolkit.

To enable LLM cleanup, put the GGUF into the models folder and uncomment
`llm_path` in your copy of the config. `[model.languages]`, `lid_path` and
`[library]` work as on a host install (`docs/server-setup.md`), with paths
under `/models`.

## 4. Binding

flow-server refuses a wildcard (`0.0.0.0`) or public bind without `[tls]`
(it would put plain `ws://` audio and tokens on every network), and drops
peers outside loopback / LAN / VPN (`docs/server-setup.md` §3, config.rs
`check_bind`). The image keeps those rules:

* **Default (`FLOW_BIND_CONTAINER_IP=1`):** the entrypoint replaces the
  config's `bind` with `127.0.0.1` and the container's own address (a
  private address on Docker and Kubernetes networks), on port `FLOW_PORT`
  (7313). Published ports and Services reach that address, the bind is
  not a wildcard, and the peer filter stays on. With Docker's default
  port publishing the server sees the real client address, so clients
  from the LAN or VPN connect and public ones are dropped. Who can reach
  the port at all is set by `FLOW_PUBLISH_ADDR`.
* **TLS (`wss://`):** create the cert as in `docs/server-setup.md` §5, put
  `cert.pem` and `key.pem` into `deploy/tls/`, uncomment the `./tls` volume
  in the compose file and the `[tls]` table in your config. The apps pin
  the certificate.
* **`FLOW_INSECURE_BIND_ANY=1`** (with `FLOW_BIND_CONTAINER_IP=0` and a
  `0.0.0.0` bind) passes `--insecure-bind-any`: no wildcard check and **no
  peer filter**. The entrypoint and the server both warn at startup. Only
  for a LAN-only host where the default does not work (an unusual container
  network); publish the port on a LAN or VPN address, never on a public one.

## 5. Health checks

`/v1/health` needs a key, and the server's auth is unchanged, so the image
checks only that the port accepts a TCP connection (`flow-healthcheck`,
used by the Docker `HEALTHCHECK` and the chart's liveness probe). It also
reports healthy while the entrypoint waits for the first key, and for
`FLOW_START_GRACE_S` (600 s) after the server process starts, because the
models load before the listener binds (84 s for Parakeet + Hebrew + LID +
Qwen on a loaded 4-core box). The chart's readiness probe is a plain TCP
check, so the Service sends traffic only once the server listens.

## 6. Helm

```sh
helm install flow deploy/helm/flow-server -n flow --create-namespace
# models: copy into the models PVC (flow-flow-server-models), e.g. through a
# temporary pod that mounts it, or use your own PVC: --set persistence.models.existingClaim=NAME
kubectl -n flow exec deploy/flow-flow-server -- flow-server keys add owner --admin
kubectl -n flow logs -f deploy/flow-flow-server
```

What the chart creates: a Deployment (one replica, `Recreate`: one writer
per state volume), PVCs for the models (mounted read-only) and the state
(both kept on `helm uninstall`), a ConfigMap with `config.toml`
(`values.config`; a change rolls the pod), a Service, and optionally an
Ingress.

| value | default | |
|---|---|---|
| `image.repository` / `image.tag` | `flow-server` / `local` | locally loaded image, `pullPolicy: Never` |
| `gpu.enabled` | `false` | CUDA image (`gpu.image`, tag `local-cuda`) + `nvidia.com/gpu: 1`; `gpu.runtimeClassName` if needed |
| `bindPodIP` | `true` | bind 127.0.0.1 + the pod IP (Binding above) |
| `insecureBindAny` | `false` | `--insecure-bind-any`; refused together with `serverTls` |
| `serverTls.enabled` / `secretName` | off | `wss://` by flow-server from a `kubernetes.io/tls` Secret; the chart adds `[tls]` |
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

## 7. Smoke test

```sh
tools/container/smoke.sh --models /path/to/models --config my-config.toml \
    --wav /path/to/models/jfk.wav:en [--build]
```

Runs its own Compose project on port 17313: the first start waits for a
key, `keys add` through `docker compose exec`, one dictation through the
WebSocket protocol, then `down` + `up` and a second dictation with the same
key. It removes the project and its state volume at the end.
