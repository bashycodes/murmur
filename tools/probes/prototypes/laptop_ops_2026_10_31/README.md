# Laptop model setup, 2026-10-31 (dated probe)

> **Status:** dated probe, ops + research. Findings:
> `docs/findings/2026-10-31-laptop-model-setup.md`.

| File | What |
|---|---|
| `rps.sh` | Runs a local `.ps1` on the laptop over ssh (`LAPTOP_SSH=user@host`), `-EncodedCommand`; prints PowerShell errors as `ERR:` |
| `fetch_models.ps1` | Fetches the 0.12.0 catalogue files (Llama-3.2-3B GGUF, zipformer-he, LID tiny-q5_1) from the pinned URLs in `apps/flow-windows/src/model_download.rs` into `%APPDATA%\FlowClone\models`, checks size + SHA-256 |
| `install.ps1` | Closes the app, installs the NSIS build with `/S` |
| `set_prefs.py` | Edits `prefs.json` (app closed): local cleanup with the 3B, cleanup order local → server → cloud, Hebrew zipformer, keyboard language, hands-free pause 0 |
| `launch.ps1` | Starts the app in the interactive session (one-shot scheduled task); `REDIRECT=1` captures stdout/stderr; lists loaded DLLs and the process's GPU memory |
| `fetch_ivrit.ps1` | ivrit-ai whisper-large-v3-turbo f16 (pinned) → `whisper-quantize` q5_0 |
| `fleurs_to_pcm16.py` | FLEURS float32 WAV → PCM16 for the clips of `../laptop_bench_2026_10_30/he_manifest.json` |
| `asr_win.py` | `../laptop_bench_2026_10_30/asr_win.py` plus a `whisper_vk` engine (whisper-cli b5454 + llama.cpp b12005's `ggml*.dll` set = Vulkan) |
| `results/asr-he-20261031.jsonl` | Per-clip + summary rows |
