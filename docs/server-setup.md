# Your own server (flow-server)

One Linux machine does the dictation for your PCs and phones and hands the
models to the apps. It listens only on your network and accepts only
loopback, LAN and VPN addresses. It never stores audio or text.

## Quick start (Docker)

```sh
docker volume create flow_data
docker run -d --name flow-server --restart unless-stopped \
  -p 0.0.0.0:7313:7313 -v flow_data:/data ghcr.io/bashycodes/murmur/flow-server:latest
docker logs -f flow-server
```

The first start downloads the models (about 1.9 GB: English, Hebrew,
language detection, AI cleanup), so it can take a few minutes. Then the log
shows the admin key, once:

```
=====================================================================
 flow-server is set up. The admin key, shown only this once:

     <43 characters>

 In each app: Settings > Your server: this machine's LAN or VPN
 address, port 7313, and the key above.
 Show the key again:  docker exec flow-server cat /data/admin.token
 One key per device:  docker exec flow-server flow-server keys add phone
=====================================================================
```

and, about a minute later, `ready … listening=…`. Stop following the log
with Ctrl+C; the server keeps running.

## Connect your app

In each app: Settings → **Your server**.

* Address: this machine's LAN IP (e.g. `192.168.1.20`) or VPN IP
  (`100.x.y.z`), without `http://` and without the port.
* Port: `7313`.
* Key: the admin key above, or better one key per device (below).

Press **Test connection**: it should say `OK`. `auth_failed` means a wrong
key; `connect_failed` means a wrong address, a stopped server or a
firewall in the way (allow TCP 7313 from your LAN/VPN).

## Everyday commands

```sh
docker exec flow-server flow-server keys add alice-phone   # a new key, shown once
docker exec flow-server flow-server keys list
docker exec flow-server flow-server keys revoke alice-phone
docker exec flow-server flow-server library list           # models the apps can download
docker pull ghcr.io/bashycodes/murmur/flow-server:latest   # update: pull, then
docker rm -f flow-server                                        # recreate with the same
                                                                # `docker run` (keys and models stay in flow_data)
```

Fewer models: add `-e FLOW_MODELS=english` to `docker run` (English only,
660 MB). Docker Compose: `docker compose -f deploy/docker-compose.yml up -d`
does the same as the commands above.

Plain `ws://` is not encrypted: use the server over a VPN or a LAN you
trust, or turn on TLS (Advanced).

## Advanced

* [`docs/container.md`](container.md): all settings (`FLOW_MODELS`,
  ports, the config in the volume), TLS, a GPU for AI cleanup, building the
  image yourself, machines without internet, Kubernetes (Helm).
* [`docs/server-advanced.md`](server-advanced.md): without Docker (binary
  and systemd, `tools/server/install.sh`), keys in detail, TLS, firewall,
  app settings, Hebrew and AI cleanup tuning, the model library, debug
  capture.
* [`docs/air-gapped.md`](air-gapped.md): a network with no internet at all.
