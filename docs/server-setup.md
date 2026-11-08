# Your own server (flow-server)

One Linux machine does the dictation for your PCs and phones and hands the
models to the apps. It listens only on your network and accepts only
loopback, LAN and VPN addresses. It never stores audio or text.

## What you need

* Docker on a Linux machine (x86_64, 4 GB RAM free).
* The Flow Clone source: `git clone` of the repo, or the source archive of
  a release (GitHub → Releases → *Source code*), unpacked. With a Flow
  Clone bundle you can skip the build (below).
* A **models folder**: one folder per model, each with a small
  `model.toml` label. Either
  * the `library/` folder of a Flow Clone bundle's speech-models zip (and
    its cleanup zip unzipped into the same place), as it is, or
  * the folder the download loop in [`air-gapped.md`](air-gapped.md)
    ("Download the models") makes.

  The server uses Parakeet (English), `zipformer-he-int8` (Hebrew),
  `whisper-tiny-lid` (language detection) and Qwen2.5 1.5B (AI cleanup)
  when they are there; only a speech model is required.

## Quick start (Docker)

From the source folder:

```sh
docker build -t flow-server -f deploy/docker/Dockerfile .
docker volume create flow_data
docker run -d --name flow-server --restart unless-stopped -p 0.0.0.0:7313:7313 \
  -v /path/to/models:/models:ro -v flow_data:/data flow-server
docker exec flow-server cat /data/admin.token
```

The build compiles the server (10-15 minutes the first time). The last
command shows the admin key: treat it like a password. The log
(`docker logs -f flow-server`) names the models it picked, then, about a
minute later, says `ready`.

**With a Flow Clone bundle:** its `docker/` folder has the image, so
there is nothing to build: `docker load -i docker/flow-server-image-<version>.tar`,
then the same `docker volume create` and `docker run` (the image is called
`flow-server`). Or unpack `docker/flow-server-source-<version>.tar.gz` and
run the `docker build` above in it.

Docker Compose does the same:
`FLOW_MODELS_PATH=/path/to/models docker compose -f deploy/docker-compose.yml up -d --build`.

## Connect your app

Find this machine's address: `hostname -I` lists it (the LAN one looks
like `192.168.1.20`, a VPN one like `100.x.y.z`).

* **Windows:** Settings → Your server. **Address**: `192.168.1.20` (no
  `ws://`, no port), and `7313` in the port box next to it. **Access
  token**: the key. **Save**, then **Test connection**: it says
  `Connected.`
* **Android:** Settings → Your server. **Host or IP**: `192.168.1.20`;
  **Port**: `7313`; **Token**: the key. Press **Test connection**.

A wrong key fails with `auth_failed`; a wrong address, a stopped server or
a firewall fails to connect.

## Everyday commands

```sh
docker exec flow-server flow-server keys add alice-phone   # a new key, shown once
docker exec flow-server flow-server keys list
docker exec flow-server flow-server keys revoke alice-phone
docker exec flow-server flow-server library list           # models the apps can download
docker logs flow-server                                    # what it is doing
```

Update: get the newer source, run the `docker build` again, then
`docker rm -f flow-server` and the same `docker run`. Keys stay in
`flow_data`; models stay in your folder.

## Safety

* `-p 0.0.0.0:7313:7313` opens the port on every IPv4 address of the
  machine. The server then refuses every peer that is not loopback, LAN or
  VPN; that is the only guard, because **ports published by Docker bypass
  `ufw`** and most host firewalls. On a machine with a public address,
  publish on its LAN or VPN address only: `-p 192.168.1.20:7313:7313`.
* Plain `ws://` is not encrypted: use the server over a VPN or a LAN you
  trust, or turn on TLS (Advanced).

## Advanced

* [`docs/container.md`](container.md): all container settings (choosing
  other models, your own config, ports), TLS, a GPU for AI cleanup,
  machines without internet, Kubernetes (Helm).
* [`docs/server-advanced.md`](server-advanced.md): without Docker (binary
  and systemd, `tools/server/install.sh`), keys in detail, TLS, firewall,
  app settings, Hebrew and AI cleanup tuning, the model library, debug
  capture.
* [`docs/air-gapped.md`](air-gapped.md): a network with no internet at all.
