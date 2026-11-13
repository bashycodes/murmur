# gpu_bench_2026_11_14

GPU latency and memory of the cleanup models, and the Qwen2.5-14B
drafting control. Findings: `docs/findings/2026-11-14-gpu-cleanup-bench.md`.
Dated probe; it has no permanent obligations.

The core runs on the Linux box (rules, prompt v2, guards, D65), the model on
the Windows PC's GPU behind llama.cpp's `llama-server` (release b12368,
Vulkan), through an SSH tunnel.

* `runner/`: `gpu-http-clean`, the 2026-11-12 runner with an HTTP cleaner
  (`src/http.rs`) in place of the in-process plugin.
* `pc/serve.ps1`, `pc/stop.ps1`, `pc/dl.ps1`: run on the PC, in
  `%USERPROFILE%\murmur-bench\` only. `serve.ps1` also samples the GPU
  memory performance counters once a second into `logs\<tag>-mem.csv`.
* `serve.sh`, `run.sh`, `batch.sh`, `pcps.sh`: drive it from Linux.
* `cpu_control.sh`: the 14B five-line control on the Linux CPU through the
  real murmur-llm plugin, drafting off and on.
* `score_gpu.py`: quality (the 2026-11-12 `score.py`, unchanged), latency,
  peak memory, sameness with the CPU run, and junk in the model's answers.
* `prefill_probe.py`: the check that found the server flags below.
* `results/`: the rows. `pc-logs/`: the PC's server logs and memory samples.

## Reproduce

```sh
P=tools/probes/prototypes/gpu_bench_2026_11_14
export PC=user@the-windows-pc
# On the PC: unpack llama-b12368-bin-win-vulkan-x64.zip (SHA-256 49a9ad97…a39e)
# into %USERPROFILE%\murmur-bench\llama-b12368\, the GGUFs into models\
# (gpu_cleanup_model_2026_11_12/fetch.sh pins; Llama 3.2 3B: pc/dl.ps1).
(cd $P/runner && cargo build --release --offline)
ssh -N -L 18091:127.0.0.1:8091 $PC &        # the tunnel
BIN=…/gpu-http-clean $P/batch.sh            # every row, about 25 min
$P/serve.sh pull
python3 -I $P/score_gpu.py gemma-3-12b llama-3.2-3b qwen2.5-7b qwen2.5-14b qwen2.5-14b-nodraft gemma-3-12b-nodraft
python3 -I $P/score_gpu.py --raw-junk cpu:qwen2.5-14b qwen2.5-14b qwen2.5-14b-nodraft cpuctl:nodraft cpuctl:draft4
```

Server flags that matter on this llama.cpp: `--swa-full --cache-ram 0
--ctx-checkpoints 0`. Without them, every Gemma request re-processed part of
its prompt through the host-RAM cache and SWA checkpoints, 4–14 s each.
