# Vulkan Hebrew whisper laptop check, 2026-11-02 (dated probe)

> **Status:** dated probe, device acceptance check for D42. Findings:
> `docs/findings/2026-11-02-vulkan-he-laptop-check.md`. Run the `.ps1` files on the laptop
> with `../laptop_ops_2026_10_31/rps.sh` (`LAPTOP_SSH=user@host`); env vars do not cross ssh,
> so prepend `$env:X='…'` lines to a copy outside the repo when a script reads one.

| File | What |
|---|---|
| `state.ps1` | Installed version, DLLs, models, bench folder, app process, `hebrew_model` |
| `install.ps1` | Backs up `prefs.json`, closes the app, installs the 1.0.0 hybrid NSIS build with `/S` |
| `setup_model.ps1` | Copies the 2026-10-31 bench's ivrit turbo q5_0 into `models\` under the catalogue name; checks size + SHA-256 |
| `plugin_bench.py` | Loads the installed `murmur_whisper_vulkan.dll` with ctypes (ABI v1, the app's load options) and times `murmur_whisper_transcribe` per clip on a resident engine; WER vs FLEURS refs. `MURMUR_WHISPER_DEVICE=cpu` = the plugin's CPU path |
| `nt_ab.ps1` | whisper-cli Vulkan (the 10-31 binary) with / without `-nt` on the 10.4 s clip |
| `cpu_ratio.ps1` | ggml-base on CPU: whisper-cli vs the plugin, to see whether the CPU slowness is model-specific |
| `app_check.ps1` | Sets `hebrew_model`, launches the app (scheduled task, stderr to a log; `DEVICE=cpu`, `NOPLUGIN=1` = temp copy without the plugin), lists loaded modules + GPU memory |
| `settings_ui.ps1`, `settings_shot.ps1` | In the interactive session: Settings → Speech to text through UI Automation, dumps the Hebrew row text (+ a screenshot that stays on the laptop) |
| `show_log.ps1`, `stop_bench.ps1`, `cleanup.ps1` | Log filter; stop only this probe's bench; remove temp files and diff prefs against the backup |
| `results/*.jsonl` | Per-clip + summary rows (FLEURS he_il dev, public) |
