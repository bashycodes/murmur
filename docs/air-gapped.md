# Running Murmur with no internet

Murmur works on a network that never reaches the internet. One machine
runs `murmur-server`; it does the dictation for every PC and phone, hands out
the speech and cleanup models, and decides who may connect (one key per
person or device). Nothing in this setup calls out: the server only
listens, and while a server is set the apps take their models from it and
from nowhere else.

You bring everything over once, on a USB stick or any other way you like.
Details live in `docs/server-advanced.md` (host install, keys, model library)
and `docs/container.md` (Docker); this page is the short path.

## Transfer checklist

On the connected machine:

1. **Get the newest release.** It must be newer than 1.0.0 (see
   `docs/findings/2026-11-04-air-gap-readiness.md` for why). A release is
   cut from `integration` with `tools/release.sh X.Y.Z --apply` and
   pushing the tag; the agent that runs this repo can do it for you. Then
   download from GitHub → Releases the files listed in [Also
   bring](#also-bring).
2. **Download the models** into a `murmur-models` folder outside the repo
   ([Download the models](#download-the-models)).
3. **Build the server** and, for Docker, save the image (§1).
4. **Check the installers** against the release's `SHA256SUMS-*.txt`
   (`sha256sum -c`), then copy to the USB stick: installers, server build
   or image, `murmur-models/`, the repo.

On the closed network:

5. Install the server (§1).
6. Copy `murmur-models/` into the server's library (§2);
   `murmur-server library list` shows 15 models, none skipped.
7. Tell the server which models to use for speech, Hebrew, language
   detection and AI cleanup, and start it (§3).
8. Create your admin key and one key per device (§4).
9. Install the apps and connect them: **Test connection** succeeds (§5).
10. On each device, download the models it needs from **From your
    server** (§5).
11. Verify: one dictation on each device in English and in Hebrew, with AI
    cleanup; then work through the "Manual checklist: real devices,
    internet blocked" in `docs/findings/2026-11-01-airgap-e2e.md`.
12. Delete any key files from the USB stick.

## Download the models

Every model any part of Murmur can use: 15 models, **7.0 GB** in all.
Each download link points at one fixed upstream version, and the apps
check every file against a SHA-256 fingerprint built into them, so a copy
made today still fits later. Take all of them: one evening of downloading,
and you never need the internet for models again.

The list, one line per file with its link, size and fingerprint, is
`docs/air-gapped-models.txt` (taken from the apps' built-in catalogue).
Paste the loop below into a `bash` shell (Linux, or WSL / Git Bash on
Windows) with `curl` and `sha256sum`. It downloads every file into `~/murmur-models` (outside the
repo), checks it, and writes the small `model.toml` label each library
folder needs (§2). Set `LIST` to where your copy of the repo is:

```sh
LIST=~/murmur/docs/air-gapped-models.txt
mkdir -p ~/murmur-models && cd ~/murmur-models && test -f "$LIST" &&
grep -v '^#' "$LIST" | grep . | {
  bad=0
  while read -r dir kind engine file size sha url title; do
    mkdir -p "$dir"
    echo "$sha  $dir/$file" | sha256sum -c --quiet - >/dev/null 2>&1 ||
      curl -fL -C - -o "$dir/$file" "$url" </dev/null
    if ! echo "$sha  $dir/$file" | sha256sum -c --quiet - >/dev/null 2>&1; then
      echo "FAILED $dir/$file"; bad=1; continue
    fi
    id=""; [ "$kind" = lid ] || id="catalogue_id = \"$dir\""
    printf 'kind = "%s"\ntitle = "%s"\nengine = "%s"\n%s\n' \
      "$kind" "$title" "$engine" "$id" > "$dir/model.toml"
  done
  if [ "$bad" = 0 ]; then echo "All models OK"; else
    echo "Some files FAILED: delete them and run this again"; false; fi
} || { echo "Stopped: check LIST, or the FAILED lines above"; false; }
```

It ends with `All models OK`; otherwise it lists each `FAILED` file and
exits with an error. Run it again to resume a cut download; files
already checked are skipped.

Each folder's name is the id the apps know the model by. "PC" is the
Windows app, "phone" the Android hybrid app; both get their models from
the server. Sizes in MB.

| Purpose | Folder | Size | Used by | |
|---|---|---|---|---|
| English | `parakeet-tdt-0.6b-v2-int8` | 661 | server speech engine, PC English pick | recommended |
| English | `parakeet-unified-en-0.6b-int8` | 663 | phone English default; server alternative | recommended |
| English | `whisper-base-en-q5_1` | 60 | phone, PC (light) | optional |
| English | `whisper-tiny-en-q5_1` | 32 | older phones | optional |
| English | `whisper-small-en-q5_1` | 190 | flagship phones, PC | optional |
| Many languages | `whisper-base` | 148 | PC | optional |
| Many languages | `whisper-base-q5_1` | 60 | phone, PC | optional |
| Many languages | `whisper-small-q5_1` | 190 | PC | optional |
| Many languages | `whisper-large-v3-turbo-q5_0` | 574 | server CUDA image or PC with a graphics chip (most accurate, any language) | optional, GPU only |
| Hebrew | `zipformer-he-int8` | 72 | server, PC and phone Hebrew default | recommended |
| Hebrew | `parakeet-tdt-0.6b-v3-he-int8` | 673 | server, PC, phone (larger alternative) | optional |
| Hebrew | `ivrit-whisper-large-v3-turbo-q5_0` | 574 | PC with a graphics chip, server CUDA image (most accurate Hebrew) | recommended with a GPU |
| Language detection | `whisper-tiny-lid` | 32 | server; the PC takes it with any Hebrew model | recommended |
| AI cleanup | `qwen2.5-1.5b-instruct-q4_k_m` | 1117 | server, PC without a GPU, flagship phones | recommended |
| AI cleanup | `llama-3.2-3b-instruct-q4_k_m` | 2019 | PC with a GPU (most accurate cleanup) | recommended with a GPU |
| AI cleanup | `gemma-3-12b-it-q4_k_m` | 7301 | server with a GPU (10 GB or more); best Hebrew | optional |
| AI cleanup | `qwen2.5-0.5b-instruct-q4_k_m` | 491 | phones, slow PCs (fastest, misses fixes) | optional |

Not on the list, and not needed:

* **Silero VAD** (`ggml-silero-v5.1.2.bin`, 0.9 MB, no fingerprint in the
  code): only if you set `vad_model_path` on the server; the built-in
  speech detection needs no model.
  `https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin`
* The links on the Android model screens and in `tools/fetch-models.sh`
  point at the newest upstream version, not a fixed one; the list above
  has the same files, fixed. `fetch-models.sh`'s default
  `ggml-tiny.en.bin` and `jfk.wav` are only for tests.

## Also bring

* **Windows installer:** `Murmur_<version>_x64-setup_hybrid.exe`. The
  `client` installer has no network code, so it can't use your server.
  Speech engines and the GPU (Vulkan) parts are inside the installer; the
  GPU needs the PC's normal graphics driver.
* **WebView2 offline installer** (215 MB), only for a Windows 10 PC that
  lacks WebView2 (Windows 11 always has it): without it the installer
  tries to download it. `https://go.microsoft.com/fwlink/?linkid=2124701`
* **Android APK:** `murmur-android-hybrid-<version>-debug.apk` (uses
  the server, falls back to the phone's own model). The `local` APK only
  for a phone that must never open a connection (§7). The release APKs are
  signed with the project's fixed debug key, not with your machine's own
  debug key: switching between a release APK and one you built yourself
  means uninstalling first, which wipes the app's settings and models.
* **Server:** the Docker image file or the host build (§1), plus the repo
  (a `git clone` or the release's source zip) for the docs, the installer
  script and the config files.
* **Building on the closed network** is not set up: the repo does not
  carry its dependencies. Bring built files. If you must build there,
  copy a machine that already builds it (Rust 1.95.0 with its download
  cache, cmake, libclang; for Android also its SDK, NDK and Gradle cache).

## 1. Install the server

Pick one. Both are built on a connected machine.

**Host install.** Build on a connected Linux machine with the same
distribution as the server (or an older one: the binary needs the same or
a newer system C library), with the AI cleanup plugin:

```sh
cargo build --release -p murmur-server --no-default-features --features whisper,parakeet,tls,llm
cargo build --release -p murmur-llm
```

Also download the sherpa-onnx 1.13.8 libraries without text to speech
(9.2 MB, the speech engine's runtime; version fixed in
`crates/murmur-asr-sherpa/Cargo.toml`; SHA-256
`bf2d998c8b07012cd5098f3b92673bc1333fd9b927767d7cb664be8190d8bc0b`) and
unpack them:
`https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib.tar.bz2`.
(The build without `no-tts` links espeak-ng, GPL-3.0; D80.)

Copy the repo folder (with `target/release/murmur-server` and
`libmurmur_llm.so`), the unpacked libraries and `murmur-models/` to the server,
then install with the speech and cleanup models:

```sh
SHERPA_ONNX_LIB_DIR=<libs>/lib tools/server/install.sh --no-build \
  --model <murmur-models>/parakeet-tdt-0.6b-v2-int8 \
  --llm-model <murmur-models>/qwen2.5-1.5b-instruct-q4_k_m/qwen2.5-1.5b-instruct-q4_k_m.gguf
```

Add `--user` for a per-user install without sudo. The installer never
downloads anything, and it writes `[model] asr_path`, `llm_path` and
`llm_lib` into a new config. Start it after §2 and §3 (`--start`, or
`systemctl restart murmur-server`).

**Docker.** On the connected machine (`docs/container.md` §8-9):

```sh
docker build -t murmur-server -f deploy/docker/Dockerfile .    # in the repo
docker save murmur-server | gzip > murmur-server.tar.gz
```

On the server: `gunzip -c murmur-server.tar.gz | docker load`. The image has
the cleanup plugin and no models; it never downloads anything. It uses
`murmur-models/` mounted as its models folder (§2).

**GPU server (optional, NVIDIA).** If the server has an NVIDIA card, also
build and save the CUDA image (`docs/container.md` "NVIDIA CUDA image"):
`docker compose --profile gpu build murmur-server-gpu`, then
`docker save murmur-server:local-cuda | gzip > murmur-server-cuda.tar.gz`. The
server then needs the NVIDIA driver and the NVIDIA container toolkit
installed (bring their packages too), and runs
`docker compose --profile gpu up -d murmur-server-gpu` instead of
`docker compose up -d`.

**Address.** With no VPN, bind the server's LAN address too
(`--bind 192.168.1.20:7313`; Docker publishes on every IPv4 address
already, `-p 0.0.0.0:7313:7313`). Plain
`ws://` is accepted only to private addresses; on a shared LAN turn on TLS
(`docs/server-advanced.md` §5) and paste the pin into the apps.

## 2. Put models in the library

The library is a folder on the server, one subfolder per model, each with a
`model.toml` label. The download loop made both: copy the folders of
`murmur-models/` in as they are.

| Install | Library folder |
|---|---|
| system | `/var/lib/murmur-server/library` |
| `--user` | `~/.local/state/murmur-server/library` |
| Docker | your `murmur-models/` folder itself, mounted at `/models` |

```sh
sudo mkdir -p /var/lib/murmur-server/library
sudo cp -r <usb>/murmur-models/. /var/lib/murmur-server/library/
# Docker: copy murmur-models/ to the server's disk (e.g. /srv/murmur-models)
# and mount it: -v /srv/murmur-models:/models:ro (docs/server-setup.md)
```

A label looks like this (`catalogue_id` = a copy of a model the apps know,
so they check its files against their built-in fingerprints and refuse a
wrong copy; leave it out for your own models):

```toml
kind = "stt"                            # stt, hebrew, llm, lid or vad
title = "Parakeet TDT 0.6B v2"          # shown in the apps
engine = "parakeet"                     # whisper, parakeet, sherpa_online, gguf
catalogue_id = "parakeet-tdt-0.6b-v2-int8"
```

More examples: `docs/server-advanced.md` §10. The server notices new folders
within a minute, no restart. Check what it serves, and why a folder was
skipped: `sudo murmur-server library list` (Docker:
`docker exec murmur-server murmur-server library list`).

## 3. Tell the server which models to use

The library only hands models to the apps. The server's own dictation uses
the models its config names. Use the library copies, so nothing is stored
twice (`<library>` is the folder from §2; in Docker `/models`):

```toml
[model]
asr_engine = "parakeet"
asr_path = "<library>/parakeet-tdt-0.6b-v2-int8"
llm_path = "<library>/qwen2.5-1.5b-instruct-q4_k_m/qwen2.5-1.5b-instruct-q4_k_m.gguf"
lid_path = "<library>/whisper-tiny-lid/ggml-tiny-q5_1.bin"   # detects Hebrew

[model.languages.he]
asr_backend = "sherpa_online"
asr_path = "<library>/zipformer-he-int8"
```

* **Host install:** the installer already wrote `asr_path`, `llm_path`
  and `llm_lib` (pointing at its own copies, which is fine). Add `lid_path`
  under `[model]` and the `[model.languages.he]` block to
  `/etc/murmur-server/config.toml` (`--user`:
  `~/.config/murmur-server/config.toml`), then `murmur-server check-config
  --config <file>` and restart.
* **Docker:** nothing to write. Start it as in `docs/server-setup.md`
  with `-v /srv/murmur-models:/models:ro`. On every start it finds the four
  models above (Parakeet, `whisper-tiny-lid`, `zipformer-he-int8`, Qwen2.5
  1.5B) in the folder and generates a config with exactly these lines; a
  missing one is left out (`docs/container.md` §3).

After the start the log's `ready` line shows `languages=he=sherpa_online`.
`docs/server-advanced.md` §2 ("LLM cleanup", "Hebrew") has the details.

## 4. Create keys and hand them out

Each person or device gets its own key. The first one, an admin key, comes
from the server's command line:

```sh
sudo murmur-server keys add owner --admin
```

(Docker made this key on its first start:
`docker exec murmur-server cat /data/admin.token` shows it.)

It prints the secret once. Paste it into your own app (Settings → Your
server: **Access token** on Windows, **Token** on Android). From then on you can manage keys from the app instead:
**Users & keys** appears in Settings on Windows and under Settings → Your
server on Android, but only for an admin key. There you create a key per
device (the secret is shown once: copy it into that device's app), see each
key's last use, dictations and model downloads, and revoke a key. A revoked
key is refused at its next connection; the others keep working. The server
won't revoke its last admin key from the app.

The same from the shell: `murmur-server keys add phone`, `keys list`,
`keys revoke phone`.

Hand secrets over the way you would a password: type them in, or move them
on the USB stick and delete the file afterwards.

## 5. Set up the apps

Install the apps from the files you carried over: the Windows installer,
and on Android the APK (allow installing from files once).

In each app open Settings → Your server: the server's address (Windows **Address**,
Android **Host or IP**; `hostname -I` on the server shows it), port `7313`, the key
(**Access token** / **Token**), and the TLS pin if you use TLS. **Test connection**
says `Connected.` on Windows.

**Windows.** Settings → Speech to text → **From your server** lists the
library. Press Download (it asks first), and the model is checked,
installed and selected; **Use** switches to one you already have. AI
cleanup and Hebrew models come the same way. With a server set, downloads
come from it (Download models from: **Your server**, the default; setup's
model steps offer it too, D79) and the app never contacts Hugging Face.

**Android, hybrid APK.** Each model screen (Speech model, Parakeet, Hebrew
model, AI cleanup) has **From your server**: tap *Show models on your
server*, then Download. A cut-off download resumes; Cancel keeps nothing.

**Android, remote APK.** Dictates on the server only; it needs no models.

## 6. No server reachable: copy models by hand

If a device can't reach the server, copy the model files to it yourself:

* **Windows:** Settings → Speech to text (or AI cleanup) → **Import from
  file…** next to a built-in model: pick its folder, a `.zip`, or the file.
  Each file is checked against the built-in fingerprint.
* **Android:** Speech model → *Import model file…*; Parakeet and Hebrew →
  *Import model folder…* or *Import .zip…* (`apps/murmur-android/INSTALL.md`
  §5).

## 6a. Bigger AI cleanup from a LiteLLM server

If the closed network has a LiteLLM proxy (or any OpenAI-compatible server)
in front of a larger model, the apps can use it for AI cleanup: Windows and
the Android hybrid APK, on the cleanup settings page, set the cloud entry's
provider to **Custom server**, enter its address (e.g. `http://10.0.0.5:4000`),
the key if it needs one, and pick a model from **Load models**. Put that entry
first in the cleanup order. Speech-to-text is not affected (D52). Over plain
`http://` the key and dictated text are not encrypted on the network.

## 7. A phone that never talks to anything: the local APK

The `local` Android flavour has no network permission and no network code
at all. It dictates on the phone only, so it needs no server or key; import
the model by hand as in §6. Pick it when the phone should never open a
connection, even to your own server.

## Checking it

`tools/e2e/airgap/run.sh` runs this whole path on one Linux machine with
no network: an admin key from the CLI, a user key from the admin API, a
model downloaded and checked, one dictation, the usage shown to the admin,
then the key revoked and refused. The last run's results:
`docs/findings/2026-11-01-airgap-e2e.md`.

