# Self-hosted server setup (flow-server)

How to run `flow-server` (Parakeet ASR) on your own Linux box and point the
Windows and Android apps at it. Design and rules: `docs/hybrid-architecture.md`
§2.6 (transport security) and §3 (server), `docs/protocol.md` §1.

The server only listens. It makes no outbound connections, never writes audio
or text to disk (unless you turn on the debug capture, §9), and accepts peers only from loopback, private LAN and
Tailscale addresses.

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
  --no-default-features --features parakeet,tls` (`parakeet,tls,llm` plus
  `-p flow-llm` with `--llm`, see "LLM cleanup" below).
* Optional, for LLM cleanup: a GGUF (`tools/fetch-models.sh --llm 1.5b` puts
  `qwen2.5-1.5b-instruct-q4_k_m.gguf` in `models/`), libclang and cmake for
  the llama.cpp build.
* Recommended: Tailscale on the server and on every client device.

## 2. Install

```sh
tools/server/install.sh --dry-run          # print every step, change nothing
tools/server/install.sh                    # system install (sudo, confirmed first)
tools/server/install.sh --user             # per-user install, no sudo
tools/server/install.sh --start            # ... and enable + start the service
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
`llm_timeout_ms`. It adds about 1.8 GB RSS and roughly 2-7 s per dictation on
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

A healthy start logs `ready workers=1 asr=parakeet load_ms=… listening=…`.
Check a config without starting: `flow-server check-config --config <path>`.

## 3. Address: Tailscale (recommended)

* Server address: `tailscale ip -4` on the server gives `100.x.y.z`. By default
  the installer binds `127.0.0.1:7313` plus that address. Without Tailscale it
  binds loopback only; add `--bind <LAN IP>:7313`.
* The unit starts after `tailscaled.service`. If the 100.x address is not up
  yet at boot, the bind fails and systemd restarts the server (`RestartSec=3`).
* Plain `ws://` is fine on Tailscale, because WireGuard already encrypts the
  traffic. The clients allow `ws://` only to private, LAN and Tailscale
  addresses. A MagicDNS name (`box.<tailnet>.ts.net`) works too, because the
  client resolves it and requires every address it gets back to be private.
* Optional: restrict port 7313 to your own devices with a Tailscale ACL.

`0.0.0.0`, `::` or a public bind address is refused unless `[tls]` is set. Even
then, peers outside loopback, LAN and Tailscale are dropped before any byte is
read. Never add `--insecure-bind-any` to the unit.

## 4. Token

* Created at install with `flow-server gen-token --out <path>`: 32 random
  bytes, base64url, mode 0600. It refuses to overwrite an existing file. To
  rotate, delete the file, run `gen-token` again, and restart the service.
* Read it once to paste into the apps: `sudo cat /etc/flow-server/token`
  (system install) or `cat ~/.config/flow-server/token` (user install).
* The system unit hands the token to the service through `LoadCredential`, so
  the dynamic service user never reads `/etc` directly.
* The token travels only in the `Authorization: Bearer` header of the upgrade
  and health requests. Neither side logs it.

## 5. TLS (optional: LAN without Tailscale)

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
sudo ufw allow in on tailscale0 to any port 7313 proto tcp          # Tailscale
sudo ufw allow from 192.168.1.0/24 to any port 7313 proto tcp       # LAN (TLS)
```

Never forward port 7313 on your router. The server is not meant to be
reachable from the internet.

## 7. App settings (Developer page, same keys on Windows and Android)

| App setting (`RouterConfig`, hybrid-architecture §5) | Value |
|---|---|
| Allow server (`allow_server`) | on |
| Prefer / order | "Server first" to use the server whenever it is up, with the local model as fallback (`allow_client` on) |
| Server host (`server.host`) | Tailscale: `100.x.y.z` (or the MagicDNS name). LAN: the LAN IP. Host only, without port or scheme |
| Port (`server.port`) | `7313` (or your `--port`) |
| Use TLS (`server.tls`) | off on Tailscale and plain LAN. On when `[tls]` is configured (forced on for public names) |
| Certificate pin (`server.tls_pin_sha256`) | the `sha256sum` above (TLS only) |
| Token (`server.token`) | the token file's content |
| Timeouts | defaults (connect 800 ms, commit base 3000 ms) |

Then press **Test connection**. It should show `OK · … · parakeet`. Hints for
the common errors:

* `auth_failed`: wrong token.
* `connect_failed`: the server isn't running, Tailscale is down, or the bind
  address is wrong.
* `proto_mismatch`: update the app or the server.

**Test session** runs one second of silence through the full route and
expects EMPTY `no_speech`.

Windows stores the token encrypted with DPAPI, and Android stores it with
Keystore. Neither puts it in exported settings.

## 8. Update or remove

* Update: pull, then run `tools/server/install.sh` again. Config and token are
  kept, and a config with `llm_path` keeps its LLM build (see "LLM cleanup").
  Then `sudo systemctl restart flow-server` (`systemctl --user restart
  flow-server` for `--user`). The installer replaces the binary while the old
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
