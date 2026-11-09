# murmur-server: Docker quick start (2026-11-08, D77)

Owner: "The murmur-server setup is way too overcomplicated. A 10 step, 500
line setup.md file proves that. I want it to be as close to a simple docker
setup as possible" (n8n as the model). After the first version (published
image, models downloaded on first start), two more decisions: "make it
buildable via Dockerfile" (no published image as the quick start) and "no
need for it to download models, just allow pointing it … to a folder
containing all the models and a metadata file".

## What changed for the person setting it up

| | before | now |
|---|---|---|
| guide | `docs/server-setup.md`, 485 lines, 10 sections | `docs/server-setup.md`, ~90 lines: build, run, read the key, connect the apps |
| steps | fetch models, build sherpa-onnx or point at its libs, run `install.sh` (sudo), `gen-token`, edit `bind`, add `lid_path` / `[model.languages.he]` / `llm_path` by hand, firewall, restart | `docker build`, `docker volume create`, `docker run` (models folder + volume), `docker exec … cat /data/admin.token` |
| Docker path before | build the image, copy models into a host folder, copy and edit `config.toml`, `docker compose up`, `docker compose exec … keys add owner --admin` | no config to copy or edit: it is derived from the models folder |
| models | manual download or copy, then name each path in the config | mount any D40 library folder (a bundle's unzipped `library/`, the `air-gapped.md` loop's folder) read-only at `/models`; the tags pick the models |
| admin key | `keys add` by hand | written to the volume on the first start; never logged; `docker exec murmur-server cat /data/admin.token` |

Moved out of the quick start: install without Docker (systemd,
`install.sh`), keys in detail, TLS, firewall, app settings, Hebrew / AI
cleanup tuning, library, debug capture → `docs/server-advanced.md` (the old
guide, sections kept so `§N` references still work). Container settings,
your own config, TLS in the container, GPU, offline images, Helm →
`docs/container.md`.

## How the config is picked

From each `/models/<id>/model.toml` (`kind`, `engine`): speech =
`parakeet-tdt-0.6b-v2-int8`, else `parakeet-unified-en-0.6b-int8`, else the
first `stt` by name; `lid`, `llm`, `hebrew` likewise with
`whisper-tiny-lid`, `qwen2.5-1.5b-instruct-q4_k_m`, `zipformer-he-int8`
preferred. Whisper and GGUF models point at the file in the folder,
sherpa-onnx ones at the folder. `MURMUR_STT` / `MURMUR_LID` / `MURMUR_LLM` /
`MURMUR_HEBREW` override (folder id or `none`). `/models` is also the
`[library] dir`, so the apps download the same copies (the hash cache is in
`/data`; the server never writes to the library folder, so `:ro` works).

## Measurements (this box, i7-6700 4C/8T, loaded by parallel agent builds)

* Image (CPU, `runtime`): 188 MB (no curl or CA certificates any more).
* Image build from source in Docker (`CARGO_BUILD_JOBS=2`): cargo step
  353 s with a cold cache, about a minute with the BuildKit cache.
* First start with the four default models mounted: `ready` 65 s after
  `docker run`.
* Dictation (jfk.wav, 11 s) through the published port: Parakeet + Qwen
  cleanup 3.2 s commit→result warm; the first, cold dictation after a
  start took 20-21 s while other builds ran (one hit the 20 s LLM timeout
  and returned the raw transcript).

## Tests

* `tools/container/smoke.sh --models DIR`: the quick start's `docker run`
  with the models folder; the key read with `docker exec` and checked
  absent from the log (first start and after restart); dictation with it;
  `keys add` + dictation with that key; restart; `library list` shows the
  4 tagged models; healthcheck healthy. Passed.
* `tools/container/entrypoint_test.sh` (no Docker; CI repo-checks): picks
  and paths, overrides and `none`, wrong-kind override, own
  `/data/config.toml` used by serve and CLI, key never in the output,
  errors (no folder, no speech model, gen-token failure, unwritable
  `/data`), Helm-style `MURMUR_CONFIG` waits for keys, `murmur-server` name.
* hadolint clean; shellcheck clean; `docker compose config` clean (both
  profiles); colleague-bundle test passes. Helm not installed here: the
  chart change (one env var) is not rendered.

## Binding and TLS

No TLS by default. The entrypoint rewrites `bind` to 127.0.0.1 + the
container's address, so the server's peer filter (loopback / LAN / VPN
only) applies to the real client address: checked, a LAN client showed as
its LAN address in the log. Over IPv6, Docker's userland proxy connects
from the bridge gateway, so the server saw `172.17.0.1` for a `[::1]`
client: a public IPv6 client would pass the filter. That is why the quick
start publishes `-p 0.0.0.0:7313:7313` (IPv4 only). On `0.0.0.0` the peer
filter is the only guard: Docker-published ports bypass `ufw`, and the
container has no systemd `IPAddressDeny`. The docs say to publish on the
LAN/VPN address on a host with a public address.

## Not done

* No published image (owner decision); CI only builds it and checks it
  starts. The CUDA image is a local `--target cuda` build.
* The colleague bundle does not carry the source; its note says to get
  the matching source (tag `v<version>`) from whoever gave the bundle.
