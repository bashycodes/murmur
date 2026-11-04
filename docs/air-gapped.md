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
and `docs/container.md` (Docker); this page is the short path. The
step-by-step move is the [transfer checklist](#transfer-checklist) at the
end.

## Download these now

Every model any part of Flow Clone can use, 15 models, **7.0 GB** in all.
Each download link is pinned to one exact upstream version, and the apps
check every file against a SHA-256 fingerprint built into them, so a copy
made today still fits the apps later. Take all of them: it costs one
evening of downloading and you never need the internet again for models.

The full list, one line per file with its link, size and fingerprint, is
`docs/air-gapped-models.txt` (copied from the Windows app's catalogue,
`apps/flow-windows/src/model_download.rs`; Android pins the same files).
On any connected machine with `bash`, `curl` and `sha256sum` (Linux, WSL,
Git Bash), run from the repo folder, this downloads everything, checks each file, and writes each
folder's `model.toml`, ready to copy into the server library (§2):

```sh
mkdir flow-models && cd flow-models
grep -v '^#' ../docs/air-gapped-models.txt | grep . |
while read -r dir kind engine file size sha url title; do
  mkdir -p "$dir"
  echo "$sha  $dir/$file" | sha256sum -c --quiet - 2>/dev/null ||
    curl -fL -C - -o "$dir/$file" "$url" </dev/null
  echo "$sha  $dir/$file" | sha256sum -c - || break
  id=""; [ "$kind" = lid ] || id="catalogue_id = \"$dir\""
  printf 'kind = "%s"\ntitle = "%s"\nengine = "%s"\n%s\n' \
    "$kind" "$title" "$engine" "$id" > "$dir/model.toml"
done
```

Run it again to resume; a file that fails its check stops the loop (delete
that file and rerun).

Rec. = recommended. Sizes are MB (10⁶ bytes). "PC" is the Windows app,
"phone" the Android hybrid app; both get the models from the server.

| Purpose | Folder (catalogue id) | Size | Used by | |
|---|---|---|---|---|
| English | `parakeet-tdt-0.6b-v2-int8` | 661 | server engine (installer default), PC English pick | rec. |
| English | `parakeet-unified-en-0.6b-int8` | 663 | server English default, phone English default | rec. |
| English | `whisper-base-en-q5_1` | 60 | phone, PC (light) | optional |
| English | `whisper-tiny-en-q5_1` | 32 | older phones | optional |
| English | `whisper-small-en-q5_1` | 190 | flagship phones, PC | optional |
| Many languages | `whisper-base` | 148 | PC, server (whisper engine) | optional |
| Many languages | `whisper-base-q5_1` | 60 | phone, PC | optional |
| Many languages | `whisper-small-q5_1` | 190 | PC | optional |
| Hebrew | `zipformer-he-int8` | 72 | server, PC and phone Hebrew default | rec. |
| Hebrew | `parakeet-tdt-0.6b-v3-he-int8` | 673 | server, PC, phone (larger alternative) | optional |
| Hebrew | `ivrit-whisper-large-v3-turbo-q5_0` | 574 | PC with a GPU (most accurate Hebrew) | rec. with a GPU |
| Language ID | `whisper-tiny-lid` (`ggml-tiny-q5_1.bin`) | 32 | server `lid_path`; the PC fetches it with any Hebrew model | rec. |
| AI cleanup | `qwen2.5-1.5b-instruct-q4_k_m` | 1117 | server cleanup, PC without a GPU, flagship phones | rec. |
| AI cleanup | `llama-3.2-3b-instruct-q4_k_m` | 2019 | PC with a GPU (most accurate cleanup) | rec. with a GPU |
| AI cleanup | `qwen2.5-0.5b-instruct-q4_k_m` | 491 | phones, slow PCs (fastest, misses fixes) | optional |

Pinned download folders (each file's full link is in the list file):

* Parakeet v2: `https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/`
* Parakeet unified: `https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/8c3a10fb13408c7a7054f6898958bf1c64a8d6c7/`
* Whisper (all `ggml-*.bin`, language ID too): `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/`
* Zipformer Hebrew: `https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he/resolve/ee769a2a13b887f91572eec349458bfb0fe79e7a/sherpa-onnx/`
* Parakeet Hebrew: `https://huggingface.co/krut42/voice-parakeet-he-int8/resolve/39de3b473bed626c7796c156d71adc66161ebfe1/`
* ivrit.ai Whisper turbo: `https://huggingface.co/tagidu/ivrit-whisper-large-v3-turbo-ggml-quantized/resolve/540d7736f31952a65efbdff0dc41e2b4b086a197/`
* Qwen2.5 0.5B / 1.5B: `https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/9217f5db79a29953eb74d5343926648285ec7e67/`, `https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/`
* Llama 3.2 3B: `https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/`

Not pinned (no fingerprint in the code), and not needed for a normal setup:

* **Silero VAD** (`ggml-silero-v5.1.2.bin`, 0.9 MB): only if you set
  `vad_model_path` on the server; the built-in voice detection needs no
  model, and the apps can't use a VAD from the library.
  `https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin`
* The links on the Android model screens and in `tools/fetch-models.sh`
  (except `--hebrew`) point at `main` or a GitHub tarball, not a fixed
  version. Use the list above instead: same files, pinned.

## Also bring

* **Windows installer:** `Murmur_<version>_x64-setup_hybrid.exe` from
  the GitHub release (the `client` installer has no network code, so it
  can't use your server). The speech engines, onnxruntime and the GPU
  (Vulkan) pieces are inside the installer; GPU use needs the PC's normal
  graphics driver.
* **WebView2** (Windows 10 only; Windows 11 has it): the app's window
  needs it, and the installer would otherwise try to download it. The
  offline installer, 215 MB: `https://go.microsoft.com/fwlink/?linkid=2124701`.
* **Android APK:** `flow-clone-android-hybrid-<version>-debug.apk` (uses
  the server, falls back to the phone's own model). Use the `local` APK
  only for a phone that must never open a connection (§6). The release
  APKs are signed with a different key than one built on your machine:
  installing one over the other means uninstalling first, which wipes the
  app's settings and models.
* **Server:** either the Docker image (`docker save`, §1), or the built
  `flow-server` plus the sherpa-onnx 1.13.8 libraries (host install, §1;
  version pinned in `crates/flow-asr-sherpa/Cargo.toml`, no fingerprint):
  `https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-linux-x64-shared-lib.tar.bz2`.
  For AI cleanup on a host install, also build the cleanup plugin on the
  connected machine (`--features whisper,parakeet,tls,llm` and
  `cargo build --release -p flow-llm`); the Docker image already has it.
* **This repo** (a `git clone` or the release's source zip): the docs, the
  installer script and `docs/air-gapped-models.txt`.
* **Rebuilding on the closed network** is not set up: the repo has no
  vendored dependencies. Bring built files instead. If you must build
  there, copy a machine that already builds it (Rust 1.95.0 with its
  `~/.cargo` cache, cmake, libclang; for Android also the SDK, NDK and
  `~/.gradle` cache), or run `cargo vendor` and follow what it prints.

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

## Transfer checklist

On the connected machine:

1. Get the newest release (GitHub → Releases): the hybrid Windows
   installer, the hybrid APK (and the local one if a phone must stay
   offline), the `SHA256SUMS-*.txt` files, and the source zip.
2. Download the models ([Download these now](#download-these-now)) into
   `flow-models/`; the loop checks every file.
3. Build the server: the Docker image and `docker save` it, or the
   host build plus the sherpa-onnx libraries (§1). Add the cleanup plugin
   if the server should do AI cleanup.
4. Bring WebView2's offline installer if any PC runs Windows 10.
5. Check the installers against `SHA256SUMS-*.txt` (`sha256sum -c`), then
   copy everything to the USB stick: installers, server build or image,
   `flow-models/`, the repo.

On the closed network:

6. Install the server (§1) and bind its LAN address (or turn on TLS).
7. Copy every folder of `flow-models/` into the server library (§2), and
   run `flow-server library list`: 15 models served, none skipped.
8. Create your admin key and one key per device (§3).
9. Install the apps and connect them (§4): **Test connection** says `OK`.
10. Download the models each device needs from **From your server** (§4).
11. Verify: one dictation on each device, in English and in Hebrew, with
    AI cleanup; then work through the manual checklist in
    `docs/findings/2026-11-01-airgap-e2e.md` ("Manual checklist: real
    devices, internet blocked").
12. Delete the key files from the USB stick.
