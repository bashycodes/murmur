# flow-server: Docker quick start (2026-11-08, D77)

Owner: "The flow-server setup is way too overcomplicated. A 10 step, 500
line setup.md file proves that. I want it to be as close to a simple docker
setup as possible" (n8n as the model: `docker volume create` + one
`docker run`).

## What changed for the person setting it up

| | before | now |
|---|---|---|
| guide | `docs/server-setup.md`, 485 lines, 10 sections | `docs/server-setup.md`, ~80 lines: 3 commands + "connect your app" |
| steps | fetch models, build sherpa-onnx or point at its libs, run `install.sh` (sudo), `gen-token`, edit `bind`, add `lid_path` / `[model.languages.he]` / `llm_path` by hand, firewall, restart | `docker volume create flow_data`, `docker run …`, `docker logs -f flow-server` |
| Docker path before | build the image locally, copy models into a host folder, copy and edit `config.toml`, `docker compose up`, `docker compose exec … keys add owner --admin` | pull the published image; nothing to copy or edit |
| models | manual download or copy | downloaded on first start (pinned, SHA-256 checked) into the volume's library, so the apps can download them from the server too |
| admin key | `keys add` by hand | printed once in the log; `docker exec flow-server cat /data/admin.token` shows it again |

Moved out of the quick start: install without Docker (systemd,
`install.sh`), keys in detail, TLS, firewall, app settings, Hebrew / AI
cleanup tuning, library, debug capture → `docs/server-advanced.md` (the old
guide, sections kept so `§N` references still work). Container settings,
TLS in the container, GPU, building the image, offline images, Helm →
`docs/container.md`.

## Measurements (this box, i7-6700 4C/8T, loaded by parallel agent builds)

* Image (CPU, `runtime`): 201 MB on disk, 51.5 MB compressed (the old
  image was 186 MB; +15 MB for curl and CA certificates).
* Image build from source in Docker (`CARGO_BUILD_JOBS=2`, cold cargo
  cache): the cargo step took 353 s; rebuild after an entrypoint change 61 s
  (BuildKit cache mounts).
* First start with `FLOW_MODELS=default` and the four models seeded into
  the volume (no download; the 1.9 GB are SHA-256 checked once): `ready`
  125 s after `docker run` (hashing + config + key + model load). Restart:
  model load 33 s (`load_ms`).
* First start with a real download (`FLOW_MODELS=whisper-tiny-en-q5_1`,
  32 MB): `ready` after 12-15 s. The default set (1.9 GB) is
  download-bound: about 3 min at 100 Mbit/s.
* Dictation (jfk.wav, 11 s) through the published port: Parakeet + Qwen
  cleanup 3.5 s commit→result warm (18-20 s on the first, cold dictation
  while other builds ran); whisper tiny.en without cleanup 0.3-0.4 s.

## Tests

* `tools/container/smoke.sh` (rewritten: tests the quick start's
  `docker run`): admin key from the log equals `/data/admin.token`,
  dictation with it, `docker exec … flow-server keys add` + dictation with
  that key, restart (key not printed again, same key works), `library
  list` shows the 4 models tagged, healthcheck healthy. Passed with the
  default set (seeded) and with a downloaded whisper model.
* `tools/container/entrypoint_test.sh` (no Docker, in CI): presets, config
  contents, key printed once, second start downloads nothing and keeps an
  edited config, seeded folders used, errors (unknown model, hash
  mismatch, no speech model, unwritable `/data`), Helm-style config waits
  for keys, `flow-server` name adds `--config`.
* hadolint clean; shellcheck clean; `docker compose config` clean (both
  profiles). Helm not installed here: the chart change (two env vars) is
  not rendered.

## Binding and TLS

No TLS by default. The entrypoint still rewrites `bind` to 127.0.0.1 + the
container's address, so the server's peer filter (loopback / LAN / VPN
only) applies to the real client address: checked, a LAN client showed as
its LAN address in the log. Over IPv6, Docker's userland proxy connects
from the bridge gateway, so the server saw `172.17.0.1` for a `[::1]`
client: a public IPv6 client would pass the filter. That is why the quick
start publishes `-p 0.0.0.0:7313:7313` (IPv4 only), not `-p 7313:7313`.
TLS needs a pin pasted into every app, the complexity being removed; it is
one config block in the volume (`docs/container.md` §6).

## Not done

* The CUDA image is not built or published by CI (a CUDA llama.cpp build
  is long and needs no runner GPU, but doubles CI time); it stays a local
  `--target cuda` build.
* The image on ghcr.io inherits the repository's visibility (private), so
  colleagues need access or the owner makes the package public.
