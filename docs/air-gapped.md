# Running Flow Clone with no internet

Flow Clone works on a network that never reaches the internet. One machine
runs `flow-server`; it does the dictation for every PC and phone, hands out
the speech and cleanup models, and decides who may connect (one key per
person or device). Nothing in this setup calls out: the server only
listens, and while a server is set the apps take their models from it and
from nowhere else.

What you need from a connected machine, once: the server build (or the
Docker image), the app installers, and the model files. Carry them over on
a USB stick or any other way you like.

Details live in `docs/server-setup.md` (host install, keys, model library)
and `docs/container.md` (Docker); this page is the short path.

## 1. Install the server

Pick one.

**Host install.** On a connected Linux machine with the same distribution
(or an older glibc) as the server, build it:

```sh
cargo build --release --offline -p flow-server --no-default-features --features whisper,parakeet,tls
```

Copy the whole repo folder (with `target/release/flow-server`), the
sherpa-onnx libraries and the Parakeet model folder to the server, then:

```sh
SHERPA_ONNX_LIB_DIR=<libs> tools/server/install.sh --no-build --model <parakeet folder> --start
```

Add `--user` for a per-user install without sudo. The installer never
downloads anything.

**Docker.** Build the image on a connected machine and move it
(`docs/container.md` §1-2):

```sh
docker save flow-server:local | gzip > flow-server-local.tar.gz   # connected machine
gunzip -c flow-server-local.tar.gz | docker load                   # server
```

Then put the models in `deploy/models/` and `docker compose up -d` as in
`docs/container.md` §3.

**Address.** With no VPN, bind the server's LAN address too
(`--bind 192.168.1.20:7313`, or `FLOW_PUBLISH_ADDR` for Docker). Plain
`ws://` is accepted only to private addresses; on a shared LAN turn on TLS
(`docs/server-setup.md` §5) and paste the pin into the apps.

## 2. Put models in the library

The library is a folder on the server, one subfolder per model (default
`<state dir>/library`: `/var/lib/flow-server/library` for the system
install, `~/.local/state/flow-server/library` for `--user`, `/models/library`
in Docker). Copy each model's files into its own folder and add a
`model.toml` that says what it is:

```sh
sudo mkdir -p /var/lib/flow-server/library/parakeet-v2
sudo cp <parakeet folder>/* /var/lib/flow-server/library/parakeet-v2/
sudoedit /var/lib/flow-server/library/parakeet-v2/model.toml
```

```toml
kind = "stt"                            # stt, hebrew, llm, lid or vad
title = "Parakeet TDT 0.6B v2"          # shown in the apps
catalogue_id = "parakeet-tdt-0.6b-v2-int8"  # only for a copy of a built-in model
engine = "parakeet"                     # whisper, parakeet, sherpa_online, gguf
```

With `catalogue_id` the apps check every file against the fingerprints
built into them, so a wrong copy is refused. Leave it out for your own
models; the apps then trust the fingerprint the server lists. More
examples (Whisper, Hebrew, AI cleanup) are in `docs/server-setup.md` §10.

The server notices new folders within a minute, no restart. Check what it
serves, and why a folder was skipped:

```sh
sudo flow-server library list
```

## 3. Create keys and hand them out

Each person or device gets its own key. The first one, an admin key, comes
from the server's command line:

```sh
sudo flow-server keys add owner --admin       # Docker: docker compose exec flow-server flow-server keys add owner --admin
```

It prints the secret once. Paste it into your own app (Settings → Your
server → Token). From then on you can manage keys from the app instead:
**Users & keys** appears in Settings on Windows and under Settings → Your
server on Android, but only for an admin key. There you create a key per
device (the secret is shown once: copy it into that device's app), see each
key's last use, dictations and model downloads, and revoke a key. A revoked
key is refused at its next connection; the others keep working. The server
won't revoke its last admin key from the app.

The same from the shell: `flow-server keys add phone`, `keys list`,
`keys revoke phone`.

Hand secrets over the way you would a password: type them in, or move them
on the USB stick and delete the file afterwards.

## 4. Set up the apps

Install the apps from the files you carried over: the Windows installer,
and on Android the APK (allow installing from files once).

In each app open Settings → Your server and fill in the server's address,
port `7313`, the key, and the TLS pin if you use TLS. **Test connection**
should say `OK`.

**Windows.** Settings → Speech to text → **From your server** lists the
library. Press Download (it asks first), and the model is checked,
installed and selected; **Use** switches to one you already have. AI
cleanup and Hebrew models come the same way. With a server set, the app
never contacts Hugging Face.

**Android, hybrid APK.** Each model screen (Speech model, Parakeet, Hebrew
model, AI cleanup) has **From your server**: tap *Show models on your
server*, then Download. A cut-off download resumes; Cancel keeps nothing.

**Android, remote APK.** Dictates on the server only; it needs no models.

## 5. No server reachable: copy models by hand

If a device can't reach the server, copy the model files to it yourself:

* **Windows:** Settings → Speech to text (or AI cleanup) → **Import from
  file…** next to a built-in model: pick its folder, a `.zip`, or the file.
  Each file is checked against the built-in fingerprint.
* **Android:** Speech model → *Import model file…*; Parakeet and Hebrew →
  *Import model folder…* or *Import .zip…* (`apps/flow-android/INSTALL.md`
  §5).

## 6. A phone that never talks to anything: the local APK

The `local` Android flavour has no network permission and no network code
at all. It dictates on the phone only, so it needs no server or key; import
the model by hand as in §5. Pick it when the phone should never open a
connection, even to your own server.

## Checking it

`tools/e2e/airgap/run.sh` runs this whole path on one Linux machine with
no network: an admin key from the CLI, a user key from the admin API, a
model downloaded and checked, one dictation, the usage shown to the admin,
then the key revoked and refused. The last run's results:
`docs/findings/2026-11-01-airgap-e2e.md`.
