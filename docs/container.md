# flow-server in a container: Advanced (Docker, Compose, Helm)

The quick start is in [`docs/server-setup.md`](server-setup.md): one
volume, one `docker run`, the admin key in the log. This page has the
settings and everything else about the image. The server inside is the same
as the host install ([`docs/server-advanced.md`](server-advanced.md)).

| file | what |
|---|---|
| `deploy/docker/Dockerfile` | CPU image (default target `runtime`) and NVIDIA CUDA image (`--target cuda`) |
| `deploy/docker/entrypoint.sh` | first-start setup (models, config, admin key), bind rewrite, CLI commands; also `/usr/local/bin/flow-server` |
| `deploy/docker/healthcheck.sh` | TCP liveness check (`flow-healthcheck`) |
| `deploy/docker-compose.yml` | Compose: the quick start as a file, `gpu` profile |
| `deploy/helm/flow-server/` | Helm chart |
| `.github/workflows/server-image.yml` | CI: builds the image on PRs, pushes `:edge` (integration) and `:X.Y.Z` + `:latest` (tags) |
| `tools/container/entrypoint_test.sh` | entrypoint unit test (no Docker) |
| `tools/container/smoke.sh` | the quick start end to end: key, dictation, `keys add`, restart |
| `tools/container/stage-prebuilt.sh` | stages host-built binaries for an offline image build |

What the image contains: `flow-server` built with the production features
(`whisper,parakeet,tls,llm`), the llama.cpp plugin `libflow_llm.so`, the
sherpa-onnx and onnxruntime shared libraries, `libgomp`, curl, and the
pinned model list (`docs/air-gapped-models.txt`). **No models**: they are
downloaded into the volume on the first start.

Images: `ghcr.io/bashycodes/murmur/flow-server`, tags `latest` and
`X.Y.Z` (releases), `edge` (the newest `integration`).

## 1. The volume: `/data`

Everything the server keeps is in one volume mounted at `/data`:

| path | what |
|---|---|
| `config.toml` | written on the first start from the models present; edit it and restart to change models, languages, tuning (every key: `crates/flow-server/dist/config.example.toml`) |
| `admin.token` | the first admin key (`token_file`, the key `owner` in `keys list`), mode 0600 |
| `library/<id>/` | the models, one folder per model with its `model.toml` (D40). The server uses them, and the apps download them from it |
| `keys.json`, `key-usage.json`, `library-hashes.json` | keys and usage (D38), the library's hash cache |

The server runs as uid/gid 10001. A Docker named volume gets that owner
by itself. A host folder instead of a volume (`-v /srv/flow:/data`) must
be owned by it: `sudo chown -R 10001:10001 /srv/flow`; the entrypoint says
so when it cannot write.

Backup: copy the volume (`docker run --rm -v flow_data:/data -v "$PWD":/b
ubuntu tar czf /b/flow_data.tgz -C /data .`). Models can be left out
(`--exclude=./library`): they download again.

## 2. Settings (environment variables)

| variable | default | |
|---|---|---|
| `FLOW_MODELS` | `default` | models downloaded on start when missing (below) |
| `FLOW_THREADS` | CPUs, at most 4 | `[model] threads` in the first config |
| `FLOW_PORT` | `7313` | port inside the container |
| `FLOW_CONTAINER_NAME` | `flow-server` | the container name in the hints the log prints |
| `FLOW_BIND_CONTAINER_IP` | `1` | see Binding |
| `FLOW_INSECURE_BIND_ANY` | `0` | see Binding |
| `FLOW_START_GRACE_S` | `600` | healthcheck grace while the models load |
| `FLOW_CONFIG` | `/data/config.toml` | a config elsewhere turns the first-start setup off (Helm) |

## 3. Models (`FLOW_MODELS`)

On every start, before the server starts, the entrypoint downloads the
models named by `FLOW_MODELS` that are not in `/data/library` yet: pinned
URLs and SHA-256 from `docs/air-gapped-models.txt` (the apps' catalogue), a
file that does not verify is an error, and an interrupted download resumes.
This is the only time anything in the container goes online; the server
itself never does.

| value | models | download |
|---|---|---|
| `default` | `parakeet-tdt-0.6b-v2-int8` (English), `whisper-tiny-lid` (language detection), `zipformer-he-int8` (Hebrew), `qwen2.5-1.5b-instruct-q4_k_m` (AI cleanup) | 1.9 GB |
| `english` | `parakeet-tdt-0.6b-v2-int8` | 0.66 GB |
| `none` | nothing (bring your own, below) | |
| folder ids | any folders of `docs/air-gapped-models.txt`, comma separated; may include `default` / `english`, e.g. `default,whisper-base-q5_1,parakeet-unified-en-0.6b-int8` | |

The first config uses the first speech (`stt`) model as the main model, and
the first language-detection, cleanup and Hebrew models of the list. The
config is written once: models added to `FLOW_MODELS` later are downloaded
and offered to the apps (library), but the server uses them only after you
edit `/data/config.toml`. To start over with a new config, delete it
(`docker exec flow-server rm /data/config.toml`) and restart.

No internet on the server: put the model folders into the volume first
(e.g. the colleague bundle's models zip, or the folders from
`docs/air-gapped.md`), then start. Files with the pinned size and hash are
used as they are:

```sh
docker run --rm -v flow_data:/data -v /path/to/library:/seed:ro --entrypoint sh \
  ghcr.io/bashycodes/murmur/flow-server:latest -c 'mkdir -p /data/library && cp -r /seed/. /data/library/'
```

Any other model (not in the list): copy its folder into
`/data/library/<id>/` with a `model.toml` (`docs/server-advanced.md` §10)
and point the config at it.

## 4. Keys

The first start writes `/data/admin.token` and prints it once. It stays
readable for you: `docker exec flow-server cat /data/admin.token`. Give
every person or device its own key (`docker exec flow-server flow-server
keys add phone`; `keys list`, `keys revoke <name>`; D38,
`docs/server-advanced.md` §4); a running server picks key changes up on the
next connection. Rotating the admin key: `docker exec flow-server
flow-server gen-token --out /data/admin.token`, then restart.

## 5. Docker Compose

`deploy/docker-compose.yml` is the quick start as a file, with a restart
policy, a read-only root filesystem, no capabilities and an 8 GB memory
cap:

```sh
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml logs -f      # the admin key, then "ready"
```

Settings from the environment or a `.env` file: `FLOW_IMAGE`,
`FLOW_MODELS`, `FLOW_PUBLISH_ADDR` (default `0.0.0.0`; set the host's LAN
or VPN address on a host with a public address), `FLOW_HOST_PORT`.
`docker compose build` builds `flow-server:local` from the source tree
(then `FLOW_IMAGE=flow-server:local`).

## 6. TLS (`wss://`)

Off by default: the server accepts only loopback, LAN and VPN peers, and a
VPN already encrypts. For a shared LAN without a VPN: create the cert as in
`docs/server-advanced.md` §5 and copy `cert.pem` and `key.pem` into the
volume's `tls/` folder (`docker cp cert.pem flow-server:/data/tls/` after
`docker exec flow-server mkdir -p /data/tls`), uncomment `[tls]` in
`/data/config.toml`, restart, and paste the certificate's pin into the
apps.

## 7. GPU (NVIDIA)

Only AI cleanup uses the GPU; the image is built locally (CI publishes the
CPU image only). See "NVIDIA CUDA image" below, then
`docker compose -f deploy/docker-compose.yml --profile gpu up -d --build flow-server-gpu`
(it needs the NVIDIA container toolkit). Both services use the
`flow_data` volume: run one at a time.

## 8. Build the image yourself

```sh
# CPU image, compiled inside Docker. Needs network: crates.io, and GitHub
# for the sherpa-onnx shared libraries (unless staged, see below).
docker build -f deploy/docker/Dockerfile -t flow-server:local .
```

The build compiles whisper.cpp and llama.cpp: about 10 minutes on a 4-core box and 3-4 GB of
RAM with the default `CARGO_BUILD_JOBS=2` (raise it with `--build-arg` on a
bigger machine). Cargo's registry and `target/` live in BuildKit cache
mounts, so a rebuild after a code change is incremental; `docker builder
prune` frees them. `--build-arg FLOW_GIT_SHA=<sha>` puts the commit into
`flow-server --version`.

### Offline build from host binaries

On a machine without network, or to reuse a host build: build on the host,
stage the files, and build the image with `SOURCE=prebuilt`. That image
build compiles nothing and downloads nothing beyond the base image
(`docker load` can provide it) when you also pass `--build-arg
WITH_CURL=0`: that image cannot download models, so fill the volume's
`library/` first (§3).

```sh
# sherpa-onnx 1.13.8 shared libs (libsherpa-onnx-c-api.so, libonnxruntime.so)
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/lib
tools/container/stage-prebuilt.sh --build     # or without --build after your own
                                              # cargo build --release (features
                                              # whisper,parakeet,tls,llm) + -p flow-llm
docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt -t flow-server:local .
```

The image is Ubuntu 24.04 (glibc 2.39): build the binaries on a host with
that glibc or older.

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

## 9. Air-gapped hosts: move the image

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

## 10. Binding

flow-server refuses a wildcard (`0.0.0.0`) or public bind without `[tls]`
(it would put plain `ws://` audio and tokens on every network), and drops
peers outside loopback / LAN / VPN (`docs/server-advanced.md` §3, config.rs
`check_bind`). The image keeps those rules:

* **Default (`FLOW_BIND_CONTAINER_IP=1`):** the entrypoint replaces the
  config's `bind` with `127.0.0.1` and the container's own address (a
  private address on Docker and Kubernetes networks), on port `FLOW_PORT`
  (7313). Published ports and Services reach that address, the bind is
  not a wildcard, and the peer filter stays on. With Docker's default
  port publishing the server sees the real client address, so clients
  from the LAN or VPN connect and public ones are dropped. Who can reach
  the port at all is set by the publish address (`-p ADDR:7313:7313`,
  Compose `FLOW_PUBLISH_ADDR`). The quick start publishes on `0.0.0.0`
  (IPv4) on purpose: Docker forwards IPv6 connections to an IPv4-only
  container through its userland proxy, so the server would see the
  proxy's private address instead of the client's and the peer filter
  could not drop a public IPv6 client.
* **TLS (`wss://`):** §6. The rewritten bind is not a wildcard either way.
* **`FLOW_INSECURE_BIND_ANY=1`** (with `FLOW_BIND_CONTAINER_IP=0` and a
  `0.0.0.0` bind) passes `--insecure-bind-any`: no wildcard check and **no
  peer filter**. The entrypoint and the server both warn at startup. Only
  for a LAN-only host where the default does not work (an unusual container
  network); publish the port on a LAN or VPN address, never on a public one.

## 11. Health checks

`/v1/health` needs a key, and the server's auth is unchanged, so the image
checks only that the port accepts a TCP connection (`flow-healthcheck`,
used by the Docker `HEALTHCHECK` and the chart's liveness probe). It also
reports healthy while the entrypoint prepares the start (model downloads,
waiting for the first key under Helm), and for
`FLOW_START_GRACE_S` (600 s) after the server process starts, because the
models load before the listener binds (84 s for Parakeet + Hebrew + LID +
Qwen on a loaded 4-core box). The chart's readiness probe is a plain TCP
check, so the Service sends traffic only once the server listens.

## 12. Helm

```sh
helm install flow deploy/helm/flow-server -n flow --create-namespace
# models: copy into the models PVC (flow-flow-server-models), e.g. through a
# temporary pod that mounts it, or use your own PVC: --set persistence.models.existingClaim=NAME
kubectl -n flow exec deploy/flow-flow-server -- flow-server keys add owner --admin
kubectl -n flow logs -f deploy/flow-flow-server
```

The chart keeps its own layout instead of the `/data` volume: it sets
`FLOW_CONFIG=/etc/flow-server/config.toml` (its ConfigMap) and
`FLOW_MODELS=none`, so the entrypoint downloads nothing, writes no config
and no admin token, and waits for the first key (`keys add` above). The
published image works too: `--set image.repository=ghcr.io/bashycodes/murmur/flow-server,image.tag=latest,image.pullPolicy=IfNotPresent`.

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

## 13. Smoke test

```sh
tools/container/smoke.sh --image flow-server:local --wav /path/to/jfk.wav:en \
    [--models default] [--seed /path/to/library]
```

Runs the quick start's `docker run` (container `flow-smoke`, volume
`flow-smoke-data`, host port 17313): the admin key from the log (checked
against `/data/admin.token`), one dictation through the WebSocket protocol
with it, `keys add` through `docker exec` and a dictation with that key,
then a restart (the key is not printed again; the same key still works).
`--seed` copies model folders into the volume first, so nothing is
downloaded. It removes the container and the volume at the end.
