# Self-hosted server setup (flow-server)

How to run `flow-server` (Parakeet ASR) on your own Linux box and point the
Windows and Android apps at it. Design and rules: `docs/hybrid-architecture.md`
§2.6 (transport security) and §3 (server), `docs/protocol.md` §1.

The server only listens. It makes no outbound connections, never writes audio
or text to disk (unless you turn on the debug capture, §9), and accepts peers only from loopback, private LAN and
VPN (100.64.0.0/10) addresses.

Running with no internet at all (server, PCs and phones on a closed
network): the short owner guide is `docs/air-gapped.md`.

## 1. Prerequisites (nothing is downloaded by the installer)

* The Parakeet model: `tools/fetch-models.sh --parakeet` (explicit, one-time
  download into `models/`), or an existing copy passed with `--model DIR`.
* Supported model folders (any sherpa-onnx NeMo transducer directory with
  `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`; int8
  files are preferred):
  * `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` (the default `install.sh`
    and `fetch-models.sh` use);
  * `sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming`
    (2026-10-23 survey, `docs/proposals/stt-model-survey.md`: same WER and
    speed as v2 on our set, no utterance drops on 43-56 s one-shot
    decodes; NVIDIA Open Model License). Point `[model] asr_path` (or
    `--model DIR`) at the folder; nothing else changes. Load-tested by
    `flow-asr-sherpa/tests/model.rs` with `FLOW_PARAKEET_UNIFIED_DIR`.
* Decoding (2026-10-23): greedy search when the session has no
  dictionary hotwords, beam search with them; an empty or implausibly short
  beam result is re-decoded greedy, and clips over 20 s are decoded in
  ≤ 20 s pieces cut at pauses (fixes the v2 beam-4 empty results and the
  long-decode sentence drops). This loads two recognizers (≈ +0.6 GB RSS);
  `ModelConfig.parakeet_decoding = "greedy"` keeps one. The earlier
  workaround, `Environment=FLOW_PARAKEET_HOTWORDS_SCORE=0` in a systemd
  drop-in (e.g. `~/.config/systemd/user/flow-server.service.d/greedy.conf`),
  can be removed once a server with this fix is installed: it still forces
  greedy-only and turns hotwords off.
* The sherpa-onnx shared libraries (`libsherpa-onnx-c-api.so`,
  `libonnxruntime.so`), passed with `SHERPA_ONNX_LIB_DIR=<dir>` or
  `--sherpa-lib <dir>`.
* A Rust toolchain. The build is `cargo build --release --offline -p flow-server
  --no-default-features --features whisper,parakeet,tls` (plus `llm` and
  `-p flow-llm` with `--llm`, see "LLM cleanup" below). `whisper`
  (whisper.cpp, needs cmake) carries language ID (`lid_path`), the Silero
  VAD and the whisper engine; `--no-whisper` leaves it out unless the
  existing config uses one of them.
* Optional, for LLM cleanup: a GGUF (`tools/fetch-models.sh --llm 1.5b` puts
  `qwen2.5-1.5b-instruct-q4_k_m.gguf` in `models/`), libclang and cmake for
  the llama.cpp build.
* Recommended: a WireGuard-based mesh VPN on the server and on every client
  device (one that hands out 100.64.0.0/10 addresses).

## 2. Install

```sh
tools/server/install.sh --dry-run          # print every step, change nothing
tools/server/install.sh                    # system install (sudo, confirmed first)
tools/server/install.sh --user             # per-user install, no sudo
tools/server/install.sh --start            # ... and enable + (re)start the service
```

| | `--system` (default) | `--user` |
|---|---|---|
| Binary | `/opt/flow-server/bin/flow-server` | `~/.local/bin/flow-server` |
| Libraries | `/opt/flow-server/lib` | `~/.local/lib/flow-server` |
| Model | `/opt/flow-server/models/<name>` | `~/.local/share/flow-server/models/<name>` |
| Config, token | `/etc/flow-server/{config.toml,token}` | `~/.config/flow-server/{config.toml,token}` |
| Unit | `/etc/systemd/system/flow-server.service`: the packaged hardened unit (`DynamicUser`, `ProtectSystem=strict`, kernel IP allowlist) with these paths | `~/.config/systemd/user/flow-server.service`: lighter hardening; the server's own bind policy and peer filter are the boundary |
| Logs | `sudo journalctl -u flow-server -f` | `journalctl --user -u flow-server -f` |

The script never overwrites an existing config or token. Every root command is
printed as `[sudo] …` and confirmed once before anything runs (`--yes` skips
the question). Other options: `--prefix`, `--bind ADDR` (repeatable),
`--port`, `--threads`, `--no-build`, `--llm`, `--llm-model FILE`. `--model`
may name the installed model folder itself (re-install in place); it is then
not copied.

### LLM cleanup (optional)

```sh
tools/server/install.sh --user --llm-model models/qwen2.5-1.5b-instruct-q4_k_m.gguf
tools/server/install.sh --user --llm       # build + install the plugin only
```

`--llm` builds flow-server with the `llm` feature (it only adds the plugin
loader) and the llama.cpp plugin `libflow_llm.so` (`-p flow-llm`, a separate
library because llama.cpp's ggml clashes with whisper.cpp's), and installs
the plugin into the libraries dir. `--llm-model FILE` implies `--llm`,
copies the GGUF next to the ASR model and writes `llm_path` and `llm_lib`
into a **new** config; an existing config is never edited, so add them to
`[model]` yourself:

```toml
llm_path = "<models dir>/qwen2.5-1.5b-instruct-q4_k_m.gguf"
llm_lib = "<libraries dir>/libflow_llm.so"
```

If the existing config already sets `llm_path`, the installer turns `--llm`
on by itself: a flow-server built without the feature refuses to start with
`llm_path` set, so a plain re-install would otherwise take the server down.
It warns when the config's `llm_lib` is not the path it installs to.

bindgen (llama-cpp-sys-2) can miss the compiler's own headers (`stdbool.h`
not found, e.g. Ubuntu with gcc 13). The installer then passes
`BINDGEN_EXTRA_CLANG_ARGS=-I$(gcc -print-file-name=include)` unless you set
that variable yourself. On a memory-tight box, `CARGO_BUILD_JOBS=4` keeps the
llama.cpp build from swapping.

The `ready` log line does not show LLM state. Check
`curl -H "Authorization: Bearer $(cat <token file>)" http://127.0.0.1:7313/v1/health`
for `"llm":true`. Cleanup runs only for sessions that ask for it
(`cleanup_level` not `none`, `raw_mode` off), within the session's
`llm_timeout_ms` plus `llm_timeout_per_char_ms` (default 15) per transcript
character, at most `llm_timeout_max_ms` (default 8000; top-level config
keys, 0 = fixed budget), since a long or Hebrew dictation decodes more
tokens. Each worker's LLM cleans one throwaway sentence at startup, so the
first dictation after a restart is not cold. It adds about 1.8 GB RSS and roughly 2-7 s per dictation on
a CPU-only i7-6700 (`docs/findings/2026-10-23-server-llm-cleanup-cost.md`).

`--user` needs a running systemd user manager. Without a login session
(ssh without `pam_systemd`, cron, an agent shell) the installer stops before
building and asks for `loginctl enable-linger $USER` (no sudo needed for your
own user on most distros; it also keeps the server running after logout). If
only `XDG_RUNTIME_DIR` points elsewhere (e.g. rootless Docker sets it), the
installer falls back to `/run/user/$UID`; for later `systemctl --user` /
`journalctl --user` calls in that shell, run them with
`XDG_RUNTIME_DIR=/run/user/$(id -u)`. Test the preflight with
`tools/server/test-install.sh`.

A healthy start logs `ready workers=1 asr=parakeet languages= load_ms=… listening=…`.
Check a config without starting: `flow-server check-config --config <path>`.

### Hebrew (optional, D33)

English stays on Parakeet; Hebrew gets its own model, used only by sessions
whose language is forced to Hebrew (the app's language set to Hebrew: `he`,
`he-IL` and `iw` all match). Auto-detect stays on the default model.

```sh
FLOW_MODELS_DIR=<models dir> tools/fetch-models.sh --hebrew   # zipformer-he int8, 69 MiB
# optional alternative, 642 MiB: --hebrew-parakeet (parakeet-tdt-0.6b-v3-he-int8)
```

The script downloads pinned revisions and checks each file's SHA-256
(`docs/findings/2026-10-30-hebrew-model-pins.md`); files already present with
the right hash are skipped, so re-running it is cheap. Without
`FLOW_MODELS_DIR` it uses `models/`. Then add to the config's `[model]`
(the installer never edits an existing config; the block is in
`crates/flow-server/dist/config.example.toml`):

```toml
[model.languages.he]
asr_backend = "sherpa_online"     # streaming zipformer (alias "zipformer")
asr_path = "<models dir>/zipformer2-66m-streaming-he-int8"
# threads = 4                     # optional, default [model] threads
```

`sherpa_online` needs the `parakeet` feature, which `install.sh` always
builds. Every worker loads the Hebrew model next to the default one at
startup (the zipformer is 69 MiB of int8 weights; the Parakeet-he
alternative costs about as much memory as the English Parakeet). After a restart the `ready` line shows
`languages=he=sherpa_online`, `check-config` prints the language route, and
`/v1/health` (`hello_ok`) lists `"languages":["he"]`. Each `result` log line
carries the decoded `language`; a Hebrew session's result has
`"language":"he"`. Measured on the i7-6700: FLEURS-dev WER 26.8 %, RTF 0.05
(`docs/findings/2026-10-28-hebrew-stt-models.md`).

## 3. Address: VPN (recommended)

* Server address: the server's VPN IPv4, `100.x.y.z` (`ip -4 addr`). By default
  the installer binds `127.0.0.1:7313` plus the first 100.64.0.0/10 address it
  finds. Without one it binds loopback only; add `--bind <LAN IP>:7313`.
* If the VPN's 100.x address is not up yet at boot, the bind fails and systemd restarts the server (`RestartSec=3`).
* Plain `ws://` is fine on a WireGuard VPN, because it already encrypts the
  traffic. The clients allow `ws://` only to private, LAN and VPN
  addresses. A VPN DNS name works too, because the
  client resolves it and requires every address it gets back to be private.
* Optional: restrict port 7313 to your own devices with your VPN's access rules.

`0.0.0.0`, `::` or a public bind address is refused unless `[tls]` is set. Even
then, peers outside loopback, LAN and VPN are dropped before any byte is
read. Never add `--insecure-bind-any` to the unit.

## 4. Keys (who may connect)

Every person or device gets its own **key** (D38): a named secret, pasted into
the app's Token field. Keys are managed with the CLI, on the server, and a
running server picks changes up at the next connection (no restart).

```sh
sudo flow-server keys add phone            # prints the secret once (stdout)
sudo flow-server keys add laptop --admin   # admin keys can manage keys from the Windows app
sudo flow-server keys list                 # name, id, admin, created, last seen, client, dictations, audio minutes
sudo flow-server keys revoke phone         # by name or id
sudo flow-server keys revoke owner --force # --force: even the last admin key
```

With an admin key saved in the Windows app, Settings → **Users & keys** does
the same from the app (D39): the key list with usage, **Create key** (the
secret is shown once, in a copy box) and **Revoke**. The server refuses to
revoke its last admin key (the app and the CLI both; only `--force` on the
CLI overrides), so you cannot lock yourself out from the app.

The binary is the installed one (`/opt/flow-server/bin/flow-server` or
`~/.local/bin/flow-server`); a Parakeet build needs
`LD_LIBRARY_PATH=<libraries dir>` (§2) like any other run, e.g.
`sudo env LD_LIBRARY_PATH=/opt/flow-server/lib /opt/flow-server/bin/flow-server keys list`.
For a user install drop `sudo` and pass `--config ~/.config/flow-server/config.toml`.

* The secret is 32 random bytes, base64url (43 characters), shown only by
  `keys add`. The server keeps only its SHA-256, so a lost secret cannot be
  read back: revoke the key and add a new one.
* Files, in the state dir (§9: `/var/lib/flow-server` for the system unit,
  `~/.local/state/flow-server` for a user install; `state_dir` in the config
  or `--state-dir` overrides it): `keys.json` (names, ids, hashes, admin
  flag, created / revoked times) and `key-usage.json` (per key: last seen,
  the app's client string, dictation count, audio milliseconds; written by
  the server every 30 s and at shutdown). Both mode 0600, replaced
  atomically. No text or audio, ever. `keys.json.lock` (empty) serialises
  the CLI and the server (admin API) when both change keys at once.
* With the config under `/etc`, `keys` defaults to `/var/lib/flow-server`
  even though root has no `$STATE_DIRECTORY`; it hands the files to the
  service user so the service can read them.
* Revoke: the next connection or health check with that key gets 401. A
  session that is already open finishes.
* The key id (`k-` + 16 hex digits) is stable and never reused; the log's
  `connected` and `result` lines carry it, never the secret.

### Upgrade from the single token

Nothing to do. The first start (or the first `keys` command) imports the
existing `token_file` as the admin key `owner`, and the apps keep using the
same token. `gen-token --out PATH` still writes a token file (fresh installs
use it for the first key); regenerating the file and restarting moves the
`owner` key to the new secret, as rotation did before. Once `owner` is
revoked it is never re-imported. A server with a `keys.json` needs no
`token_file` at all.

* Read the owner token to paste into the apps: `sudo cat /etc/flow-server/token`
  (system install) or `cat ~/.config/flow-server/token` (user install).
* The system unit hands the token file to the service through
  `LoadCredential`, so the dynamic service user never reads `/etc` directly.
* Keys travel only in the `Authorization: Bearer` header of the upgrade
  and health requests. Neither side logs them.

## 5. TLS (optional: LAN without a VPN)

Use `wss://` when the clients reach the server over a LAN that WireGuard
doesn't cover. Self-signed certificates are pinned by the SHA-256 of the leaf
certificate DER.

```sh
# on the server: a self-signed P-256 cert for the address the apps will use
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 825 \
  -subj "/CN=flow-server" -addext "subjectAltName=IP:192.168.1.20" \
  -keyout key.pem -out cert.pem
# the pin to paste into the apps (64 hex)
openssl x509 -in cert.pem -outform DER | sha256sum | cut -d' ' -f1
```

* User install: put both files in `~/.config/flow-server/` (key mode 0600) and
  uncomment `[tls] cert/key` in the config.
* System install: the dynamic service user can't read a root-only key.
  1. Put both files in `/etc/flow-server/` (key mode 0600).
  2. Add `LoadCredential=tls-cert:/etc/flow-server/cert.pem` and
     `LoadCredential=tls-key:/etc/flow-server/key.pem` with
     `sudo systemctl edit flow-server`.
  3. Set `cert = "/run/credentials/flow-server.service/tls-cert"` and
     `key = "/run/credentials/flow-server.service/tls-key"` under `[tls]`.

  `check-config` run by hand then reports the credential paths as missing;
  the service itself sees them.

## 6. Firewall

The system unit already confines traffic in the kernel: `IPAddressDeny=any`
plus allow loopback, RFC 1918, 100.64.0.0/10, link-local and fc00::/7. A host
firewall is a second layer. With ufw:

```sh
sudo ufw allow in on <vpn-iface> to any port 7313 proto tcp        # VPN
sudo ufw allow from 192.168.1.0/24 to any port 7313 proto tcp       # LAN (TLS)
```

Never forward port 7313 on your router. The server is not meant to be
reachable from the internet.

## 7. App settings (Developer page, same keys on Windows and Android)

| App setting (`RouterConfig`, hybrid-architecture §5) | Value |
|---|---|
| Allow server (`allow_server`) | on |
| Prefer / order | "Server first" to use the server whenever it is up, with the local model as fallback (`allow_client` on) |
| Server host (`server.host`) | VPN: `100.x.y.z` (or its VPN DNS name). LAN: the LAN IP. Host only, without port or scheme |
| Port (`server.port`) | `7313` (or your `--port`) |
| Use TLS (`server.tls`) | off on a VPN and plain LAN. On when `[tls]` is configured (forced on for public names) |
| Certificate pin (`server.tls_pin_sha256`) | the `sha256sum` above (TLS only) |
| Token (`server.token`) | a key's secret (`keys add`), or the token file's content for the `owner` key |
| Timeouts | defaults (connect 800 ms, commit base 3000 ms) |

Then press **Test connection**. It should show `OK · … · parakeet`. Hints for
the common errors:

* `auth_failed`: wrong or revoked key.
* `connect_failed`: the server isn't running, the VPN is down, or the bind
  address is wrong.
* `proto_mismatch`: update the app or the server.

**Test session** runs one second of silence through the full route and
expects EMPTY `no_speech`.

Windows stores the token encrypted with DPAPI, and Android stores it with
Keystore. Neither puts it in exported settings.

## 8. Update or remove

* Update: pull, then run `tools/server/install.sh` again (`--start` restarts
  the service for you). Config, token, keys and models are kept; without
  `--model` the config's `asr_path` is reused in place. A config with
  `llm_path` keeps its LLM build, one with `lid_path` / `vad_model_path` /
  a whisper engine its whisper build. Without `--start`, `sudo systemctl
  restart flow-server` (`systemctl --user restart flow-server` for `--user`). The installer replaces the binary while the old
  one keeps running, so the downtime is only the restart and model load.
* Remove (system): `sudo systemctl disable --now flow-server`, then
  `sudo rm -r /opt/flow-server /etc/flow-server /etc/systemd/system/flow-server.service`,
  then `sudo systemctl daemon-reload`.

## 9. Debug capture (optional, beta)

Off by default. When a dictation that went through the server comes out
wrong, the server can keep what it received so the session can be replayed
on the PC (flow-cli), like the Android app's "Keep last dictation audio".

```toml
[debug]
capture = true
max_files = 30   # newest WAVs kept (default 30)
max_age_h = 72   # older ones deleted (default 72)
# dir = "/var/lib/flow-server/debug"   # default: <state dir>/debug
```

* State dir: `$STATE_DIRECTORY` (the system unit's `StateDirectory=`, i.e.
  `/var/lib/flow-server`), else `$XDG_STATE_HOME/flow-server`, else
  `~/.local/state/flow-server` (user install).
* Layout, same as the phone's `files/debug/`:
  `debug/audio/<ts>.wav` (16 kHz mono PCM16, every sample the client sent,
  named by the session start in UTC with colons as dashes, e.g.
  `2026-10-27T10-15-30.123Z.wav`) and `debug/dictations.jsonl`, one line per
  session: `ts`, `audio` (the WAV name), `route` (`server`), `engine`,
  `model`, `raw` (transcript), `text` (after AI cleanup), `status`,
  `empty_reason`, `fallback_reason`, `cleanup_route`, the core timings (`audio_ms`,
  `asr_ms`, `llm_ms`, `post_ms`, `total_ms`, …), `queue_ms`,
  `start_to_commit_ms` and the peer IP.
* Never written: the token, the client's `Authorization` header, any API
  key, the field context or the dictionary. Nothing is uploaded.
* Files are mode 0600, directories 0700. The newest `max_files` WAVs are
  kept and none older than `max_age_h`; a line goes when its WAV goes.
  Pruning runs after every save and at server start.
* `check-config` validates the section (`max_files` and `max_age_h` ≥ 1,
  `dir` absolute) and prints the capture directory when it is on. Startup
  logs a warning while it is on.
* To stop: set `capture = false` and restart; delete the `debug` folder to
  remove what was kept.

## 10. Model library (serve models to the apps, D40)

The server can hand model files to the apps, so a PC or phone with no
internet gets its models from your own server. Nothing is downloaded from
anywhere: you copy model folders into the library folder by hand and tag
each one; the server lists the tagged ones and serves their files on the
same port and TLS as dictation, to any valid key.

* Folder: `[library] dir` in the config, default `<state dir>/library`
  (`/var/lib/flow-server/library` for the system unit,
  `~/.local/state/flow-server/library` for a user install). It must be
  readable by the service; the system unit can read its state dir, but not
  `/home` (`ProtectHome=yes`).
* One folder per model. The folder name is the model's id (only
  `A-Z a-z 0-9 . _ -`). A single-file model (a GGUF, a whisper `.bin`) still
  gets its own folder. Subfolders are allowed (up to 4 deep); hidden files
  (`.name`) are ignored.
* Each folder needs a `model.toml` tag. Folders without one, or with an
  invalid one, are skipped; the log says why when the set changes, and so
  does `flow-server library list`.
* Changes are picked up without a restart: the server rescans every
  `rescan_s` seconds (default 60; one `stat` per file). A file is hashed
  (SHA-256) only when it is new or its size or mtime changed; the hashes are
  cached in `<state dir>/library-hashes.json`. A model shows up once all its
  files are hashed (a few seconds per GB, in the background).
* Symlinks inside the library are followed; a symlink that leads out of it
  skips the model. Only files the scan listed can be downloaded.
* Each download counts toward the key that made it (`keys list` →
  DOWNLOADS: requests that sent bytes, and the bytes).

```sh
sudo mkdir -p /var/lib/flow-server/library/whisper-base
sudo cp ggml-base.bin /var/lib/flow-server/library/whisper-base/
sudoedit /var/lib/flow-server/library/whisper-base/model.toml
sudo flow-server library list   # what is served, and why folders were skipped
```

### The tag: `model.toml`

| Key | Required | Meaning |
|---|---|---|
| `kind` | yes | `stt` (speech to text, every language), `hebrew` (the Hebrew route), `llm` (AI cleanup), `lid` (language detection), `vad` (voice activity) |
| `title` | yes | Shown in the apps |
| `language` | no | `en`, `he`, …; leave out for a multilingual model |
| `catalogue_id` | no | The app catalogue id this folder is a copy of (`whisper-base`, `parakeet-tdt-0.6b-v2-int8`, …). The apps then check the files against the hashes built into them |
| `engine` | no | How the app loads it: `whisper` (whisper.cpp ggml `.bin`), `parakeet` (sherpa-onnx NeMo transducer), `sherpa_online` (sherpa-onnx streaming transducer), `gguf` (llama.cpp), or another short name |

Unknown keys are an error (a typo would otherwise vanish silently).

```toml
# whisper-base/model.toml  (next to ggml-base.bin)
kind = "stt"
title = "Whisper base"
catalogue_id = "whisper-base"
engine = "whisper"
```

```toml
# zipformer-he/model.toml  (next to encoder/decoder/joiner .onnx + tokens.txt)
kind = "hebrew"
title = "Zipformer Hebrew"
language = "he"
catalogue_id = "zipformer-he-int8"
engine = "sherpa_online"
```

```toml
# qwen2.5-1.5b/model.toml  (next to the .gguf)
kind = "llm"
title = "Qwen 2.5 1.5B (cleanup)"
catalogue_id = "qwen2.5-1.5b-instruct-q4_k_m"
engine = "gguf"
```

```toml
# whisper-tiny-lid/model.toml  (next to ggml-tiny-q5_1.bin)
kind = "lid"
title = "Whisper tiny (language detection)"
engine = "whisper"
```

```toml
# silero-vad/model.toml  (next to silero_vad.onnx)
kind = "vad"
title = "Silero VAD"
engine = "silero"
```

The wire format (`GET /v1/models`, downloads with `Range`) is in
`docs/protocol.md` §1.

### What the apps do with it (D41)

With this server set, the Windows app downloads models **only** from the
library (Settings → Speech to text → From your server): it never contacts
Hugging Face, even for a model the library lacks. Folders with a
`catalogue_id` must match the hashes built into the app (a different copy
is refused); other folders are trusted by the hash the server lists. `kind`
+ `engine` decide what a model is used for: `stt` + `whisper` / `parakeet`,
`hebrew` + `sherpa_online` / `parakeet`, `llm` + `gguf`, `lid` + `whisper`.
`vad` and other engines are listed with the reason and can't be chosen.
Interrupted downloads resume. The Android hybrid app does the same from
each model screen (*From your server*).
