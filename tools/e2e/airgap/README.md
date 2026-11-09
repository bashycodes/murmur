# Air-gapped end-to-end check

`run.sh` checks the no-internet path of `docs/air-gapped.md` on one Linux
machine, with the real server binary and the apps' own client code.

```sh
tools/e2e/airgap/run.sh --models <folder with ggml-tiny.en.bin and jfk.wav>
```

`tools/fetch-models.sh` (no arguments) fetches both files once:
`ggml-tiny.en.bin` (whisper.cpp) and `jfk.wav` (the whisper.cpp sample, a
public-domain JFK speech excerpt). If `ggml-tiny.bin` is there too, it
becomes a second library model.

## What it does

1. Builds `murmur-server` (release, default features `whisper,tls`) and the
   client `crates/murmur-remote/examples/airgap_e2e.rs` (`--no-build` skips
   this; `CARGO_TARGET_DIR` and `BUILD_PREFIX` apply).
2. Makes a temp dir with a state dir, a config (bind `127.0.0.1:17399`,
   real whisper tiny.en engine read from `--models`), and a library with
   copies of the model files tagged by `model.toml` (one `stt`, one `lid`)
   plus an untagged folder that must not be served. Prints
   `murmur-server library list`.
3. Runs the rest in a loopback-only network namespace (`unshare -rn`) when
   the system allows unprivileged user namespaces; otherwise on loopback
   only. Either way the server and the client run under
   `strace -e trace=connect,sendto,sendmsg`.
4. `murmur-server keys add e2e-admin --admin` (CLI) → start the server →
   the client:
   * admin `whoami` is admin;
   * admin API creates the user key `e2e-phone`;
   * the user key lists the library and downloads every file through
     `LibraryClient` (fingerprint-checked), hashed again against the source;
   * the user key gets `forbidden` from the admin API;
   * one dictation of the clip over the user key (`RemoteBackend`, the
     apps' WebSocket client), transcript must contain `--expect-text`;
   * the admin's key list shows the user's dictations, audio, downloads,
     download bytes and client;
   * revoke → the user key is refused by the admin API, the library and
     the dictation stream; the admin key still works.
5. Stops the server, then `murmur-server keys list` must show `e2e-phone`
   revoked with its usage kept on disk.
6. Network audit: every inet address in the strace logs must be loopback.

Each step prints `PASS`/`FAIL`; the exit code is 0 only when all pass.
The temp dir is deleted at the end (`--keep` keeps it). Key secrets are in a
mode-0600 temp file and the client's memory only; nothing prints them.

Results of the last run: `docs/findings/2026-11-01-airgap-e2e.md`.
