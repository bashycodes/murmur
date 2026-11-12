# Murmur {{VERSION}}: install notes

Murmur is a dictation app: you press a shortcut, speak, and the text
is typed into whatever app you are in. Speech is turned into text on your
own PC or phone, or on a small server you run yourself. Nothing goes to
the internet unless you turn on the optional cloud settings.

This bundle: version {{VERSION}}, build `{{COMMIT}}`, variant `{{VARIANT}}`.

## What is in the bundle

| File | What it is |
|---|---|
| `windows/{{WIN_EXE}}` | Windows 10/11 installer (64-bit). Works on its own and with a server. |
| `android/{{APK}}` | Android app (Android 13 or newer). Works on its own and with a server. |
<!-- image -->
| `docker/{{IMAGE_TAR}}` | The `murmur-server` Docker image, ready for `docker load` (see [The server](#the-server-optional-for-a-team)). |
<!-- /image -->
| `docker/{{SOURCE_TGZ}}` | The server's source code, to build the Docker image yourself. |
| `server/` | `murmur-server` for Linux x86_64 without Docker: the program, its libraries, a config and a systemd service. With Docker you do not need it. |
| `{{MODELS_ZIP}}` | The speech models (English, many languages, Hebrew, language detection{{DIARIZER_LIST}}). |
| `{{CLEANUP_ZIP}}` | The AI cleanup model (fixes punctuation, fillers, "scratch that"). Optional. |
| `SHA256SUMS.txt` | Fingerprints of the files in this folder; each models zip has its own inside. Check with `sha256sum -c SHA256SUMS.txt`. |

You do not need everything. One person on one PC needs only the Windows
installer and two or three models. A team shares one server and installs
the apps.

## Windows

1. Run `{{WIN_EXE}}`. For a silent install (no questions, e.g. from a
   script or software deployment): `{{WIN_EXE}} /S`.
2. Windows SmartScreen may warn that the publisher is unknown (the
   installer is not code-signed): *More info* → *Run anyway*.
3. The app starts in the tray. Open it and go to Settings → Speech to
   text. Pick a model and press **Import from file…**, then choose that
   model's folder from the unzipped models (see [Models](#models)). Each
   file is checked against a fingerprint built into the app.
4. Hold **Right Alt** (the default push-to-talk key; change it in
   Settings → General → Shortcuts), speak, let go.

Windows 10 PCs without WebView2 download it during install; Windows 11
always has it.

<!-- notetaker -->
### Meetings (Notetaker)

This build records and transcribes meetings in any call app (it records
your microphone and what the PC plays): Meetings page → *Start meeting
recording*. To tell the
other people on a call apart, import the **Speaker separation** model:
Settings → Speech to text → *Speaker separation* → **Import from file…** →
the `speaker-diarization` folder from the models zip. Without it the
transcript only says "Me" and "Others". Recording a call may need the
other participants' consent where you are; ask them.
<!-- /notetaker -->

## Android

1. Copy `{{APK}}` to the phone and open it in Files. Allow *Install
   unknown apps* for Files when Android asks.
2. Open Murmur, menu ☰ → **Permissions**, and tap each row:
   microphone, display over other apps (the bubble), accessibility
   service, battery optimisation. If the accessibility switch is greyed
   out ("restricted setting"): Android Settings → Apps → Murmur → ⋮
   → *Allow restricted settings*, then try again.
3. Import a speech model: Murmur → **Speech model** → *Import
   model folder…* (Parakeet) or *Import model file…* (whisper),
   and pick the files from the unzipped models. For Hebrew: Settings →
   Speech to text → **Hebrew model** → *Import model folder…*. With a
   server set (below) you can instead tap *From your server* on those
   pages and download from it.

Updates: every Murmur APK is signed with the same key, so a newer APK
installs over this one and keeps your settings, dictionary and models.
Uninstalling deletes them.

## Models

Unzip `{{MODELS_ZIP}}` (and `{{CLEANUP_ZIP}}` if you want AI cleanup).
Both unpack into one `library/` folder with one subfolder per model; each
folder's name is the model's id, and its `model.toml` labels it for the
server.

| You want | Model folder | Size |
|---|---|---|
| English on a PC or the server | `parakeet-tdt-0.6b-v2-int8` | 661 MB |
| English on a phone | `parakeet-unified-en-0.6b-int8` (or `whisper-base-en-q5_1`, 60 MB, on older phones) | 663 MB |
| Other languages | `whisper-base-q5_1` (light) or `whisper-small-q5_1` (more accurate) | 60 / 190 MB |
| Hebrew | `zipformer-he-int8`, plus `whisper-tiny-lid` to detect Hebrew automatically | 72 + 32 MB |
| Hebrew, PC or server with a graphics card | `ivrit-whisper-large-v3-turbo-q5_0` (most accurate Hebrew) | 574 MB |
| Any language, PC or server with a graphics card | `whisper-large-v3-turbo-q5_0` (stock Whisper turbo, most accurate multilingual) | 574 MB |
| AI cleanup | `qwen2.5-1.5b-instruct-q4_k_m` (from the cleanup zip) | 1117 MB |
<!-- notetaker -->
| Meeting speakers (Windows) | `speaker-diarization` (at the top of the models zip, not in `library/`) | 34 MB |
<!-- /notetaker -->

The other folders are alternatives (smaller, faster or English-only
whisper models, a second Hebrew model). Each zip's `SHA256SUMS.txt` lists
its files with their fingerprints: `sha256sum -c SHA256SUMS.txt` in the
unzipped folder checks them.

## The server (optional, for a team)

One Linux machine (x86_64; 4 cores and 8 GB RAM are plenty) does the
dictation for everyone and hands the models to the apps. It only listens
on your network; it never connects anywhere.

### With Docker (recommended)

First get the `murmur-server` image, from this folder:

<!-- image -->
```sh
docker load -i docker/{{IMAGE_TAR}}
```

Or build it yourself from the source next to it (10-15 minutes; needs
internet for the build):

<!-- /image -->
<!-- noimage -->
This bundle has no prebuilt image: build it from the source in `docker/`
(10-15 minutes; needs internet for the build):

<!-- /noimage -->
```sh
tar -xzf docker/{{SOURCE_TGZ}}
docker build -t murmur-server -f murmur-server-source-{{VERSION}}/deploy/docker/Dockerfile murmur-server-source-{{VERSION}}
```

Then unzip the models and start it:

```sh
unzip -q /path/to/{{MODELS_ZIP}} 'library/*' -d ~/murmur-models
unzip -q /path/to/{{CLEANUP_ZIP}} 'library/*' -d ~/murmur-models   # optional: AI cleanup
docker volume create murmur_data
docker run -d --name murmur-server --restart unless-stopped -p 0.0.0.0:7313:7313 \
  -v ~/murmur-models/library:/models:ro -v murmur_data:/data murmur-server
docker exec murmur-server cat /data/admin.token
```

The server uses the bundle's `library/` folder as it is: it picks
Parakeet, Hebrew, language detection and (with the cleanup zip) AI cleanup
from it, and offers every model in it to the apps. The last command shows
the admin key (treat it like a password); about a minute later
`docker logs murmur-server` says `ready`. In each app, Settings → Your
server: this machine's address (`hostname -I` shows it), port 7313, the
key. Keys, one per person or device (shown once):

```sh
docker exec murmur-server murmur-server keys add alice-laptop --admin   # admin keys can manage keys from the Windows app
docker exec murmur-server murmur-server keys add alice-phone
docker exec murmur-server murmur-server keys list
docker exec murmur-server murmur-server keys revoke alice-phone
```

#### With an NVIDIA graphics card

The server can run Whisper on the graphics card: build the CUDA image
instead (needs internet and the NVIDIA container toolkit on the machine;
20-40 minutes):

```sh
docker build -t murmur-server:cuda --target cuda \
  -f murmur-server-source-{{VERSION}}/deploy/docker/Dockerfile murmur-server-source-{{VERSION}}
```

Then start it as above with `--gpus all` and `murmur-server:cuda`, and
pick one or both turbo models (both are in the models zip):

```sh
docker run -d --name murmur-server --restart unless-stopped --gpus all -p 0.0.0.0:7313:7313 \
  -e MURMUR_HEBREW=ivrit-whisper-large-v3-turbo-q5_0 \
  -e MURMUR_STT=whisper-large-v3-turbo-q5_0 \
  -v ~/murmur-models/library:/models:ro -v murmur_data:/data murmur-server:cuda
```

* `MURMUR_HEBREW=ivrit-whisper-large-v3-turbo-q5_0`: Hebrew on ivrit.ai's
  Whisper turbo (the most accurate Hebrew model) instead of the zipformer.
* `MURMUR_STT=whisper-large-v3-turbo-q5_0`: stock Whisper turbo as the
  speech model for English and every other language, instead of Parakeet.
  Leave this line out to keep Parakeet for English (fast on any machine);
  keep it in if you dictate in several languages.

`docker logs murmur-server` says where each Whisper model runs
(`whisper runs on the GPU (CUDA: ...)`). Without a usable graphics card it
says why and runs on the processor, which works but takes about 13 s per
dictation with turbo: then drop both lines. More: `docs/container.md` §7.

Ports published by Docker bypass `ufw`: the server itself refuses
everything except loopback, LAN and VPN addresses. On a machine with a
public address, publish on its LAN address only
(`-p 192.168.1.20:7313:7313`). More: `docs/server-setup.md` and
`docs/container.md` in the source archive.

### With a GPU: a larger cleanup model

If the server has an NVIDIA GPU with 10 GB of memory or more, use
**Gemma 3 12B** for AI cleanup. It is the most careful model for Hebrew and
as good as the default in English (`docs/model-benchmarks.md`, "GPU cleanup
model"). On a CPU it is far too slow, at about 6 s per dictation.

You need the NVIDIA driver and the NVIDIA container toolkit on the host.

1. Put the model in the library. It is 7.3 GB. Skip this if the bundle was
   built with `--llm all`, because the model is then already in the
   cleanup zip.

   ```sh
   d=~/murmur-models/library/gemma-3-12b-it-q4_k_m; mkdir -p $d
   curl -L -o $d/google_gemma-3-12b-it-Q4_K_M.gguf \
     https://huggingface.co/bartowski/google_gemma-3-12b-it-GGUF/resolve/648e3a36a77c8a9f12d86e741f9dcb9089c769c4/google_gemma-3-12b-it-Q4_K_M.gguf
   echo "fc57f67efa46d711c346e587cbef7d049e95f3df8db2eb2271153343ef0acc7b  $d/google_gemma-3-12b-it-Q4_K_M.gguf" | sha256sum -c
   printf 'kind = "llm"\ntitle = "Gemma 3 12B Instruct"\ncatalogue_id = "gemma-3-12b-it-q4_k_m"\n' > $d/model.toml
   ```

2. Build the CUDA image from the source. Set `CUDAARCHS` to your GPU's
   compute capability, for example 86 for an RTX 30xx.

   ```sh
   docker build --target cuda --build-arg CUDAARCHS="86" -t murmur-server:cuda \
     -f murmur-server-source-{{VERSION}}/deploy/docker/Dockerfile murmur-server-source-{{VERSION}}
   ```

3. Start the server with the GPU and the model.

   ```sh
   docker run -d --name murmur-server --restart unless-stopped --gpus all -p 0.0.0.0:7313:7313 \
     -e MURMUR_LLM=gemma-3-12b-it-q4_k_m \
     -v ~/murmur-models/library:/models:ro -v murmur_data:/data murmur-server:cuda
   ```

The first start loads the model, which takes a few minutes. Then
`docker logs murmur-server` names `AI cleanup gemma-3-12b-it-q4_k_m`.

Gemma is under the Gemma Terms of Use (NOTICE). Anyone who passes the
model on must include those terms.

Choose by GPU memory:

* 6–8 GB: use `MURMUR_LLM=llama-3.2-3b-instruct-q4_k_m`. It is in the
  cleanup zip with `--llm all`.
* Less than 6 GB, or no GPU: keep the default.

### Without Docker

For a server without Docker ({{GLIBC}} or newer, e.g. Ubuntu 24.04):
{{SERVER_NOTE}}

Install (as root, from the bundle's `server/` folder):

```sh
sudo mkdir -p /opt/murmur-server /etc/murmur-server /var/lib/murmur-server/library
sudo cp -r bin lib /opt/murmur-server/
sudo install -m 0755 murmur-server.sh /usr/local/bin/murmur-server
sudo install -m 0644 config.toml /etc/murmur-server/config.toml
sudo install -m 0644 murmur-server.service /etc/systemd/system/murmur-server.service
sudo unzip -q <path>/{{MODELS_ZIP}} 'library/*' -d /var/lib/murmur-server/
sudo unzip -q <path>/{{CLEANUP_ZIP}} 'library/*' -d /var/lib/murmur-server/   # optional: AI cleanup
sudo murmur-server gen-token --out /etc/murmur-server/token
```

The token in `/etc/murmur-server/token` is the server's first admin key,
named `owner` in `murmur-server keys list`. Keep it for yourself; give
everyone else their own key.

Then edit `/etc/murmur-server/config.toml`:

* `bind`: add the server's LAN or VPN address, e.g.
  `bind = ["127.0.0.1:7313", "10.1.2.3:7313"]`. Only private (LAN/VPN)
  addresses are accepted; never expose the port to the internet.
* Without the cleanup zip, delete the `llm_path` line.

Check and start it:

```sh
sudo murmur-server check-config --config /etc/murmur-server/config.toml
sudo systemctl daemon-reload && sudo systemctl enable --now murmur-server
sudo journalctl -u murmur-server -f      # wait for the "ready" line
sudo murmur-server library list          # the models the apps can download
```

Keys: the same `keys` commands as above, as `sudo murmur-server keys …`.
Allow TCP port 7313 from your LAN/VPN in the server's firewall.

## Connect the apps to the server

In each app: Settings → **Your server**. Enter the server's address
(`hostname -I` on the server shows it; no `http://`, no port) in
**Address** (Windows) or **Host or IP** (Android), port `7313`, and the
key in **Access token** (Windows) or **Token** (Android). Press **Test
connection**: Windows says `Connected.` Dictation then runs on the server, and falls back to
the device's own model when the server is unreachable (if one is
imported). *From your server* on the model pages downloads models from
the server's library.

* `auth_failed`: wrong or revoked key.
* `connect_failed`: server not running, wrong address, or the firewall.

## Known limits

* Test builds: the Windows installer is not code-signed, the Android APK
  is a debug build. Expect rough edges and report them.
* The apps never download models from the internet by themselves (except
  Windows when you press Download in Settings); import them or use the
  server.
* The server has no web page; manage it with the commands above or from
  the Windows app (Settings → Users & keys, admin keys only).
* Plain `ws://` is unencrypted: use it on a VPN or a trusted LAN only.
  For a shared LAN, see the TLS section of the project's server docs.
* Speech models are English, Hebrew and whisper's languages; accuracy
  for other languages is whisper's.
<!-- notetaker -->
* Meetings (Notetaker) is a preview: Windows only, and it records only
  while the app runs.
<!-- /notetaker -->
