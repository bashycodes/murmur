# Server-side LLM cleanup: cost on the home server (2026-10-23)

**Question.** What does turning on LLM cleanup (`llm_path`, Qwen2.5-1.5B-Instruct Q4_K_M)
on the home `flow-server` (i7-6700, 8 threads, no GPU, Parakeet unified 0.6B int8,
`workers = 1`, `threads = 4`) add to release→text, and is it worth leaving on?

**Answer.** It is not worth it on this box today. **The server is back on the pre-LLM config.**

* With the Android budget (`llm_timeout_ms = 3000`), cleanup only finished on the two
  shortest clips (5.8 s, 11 s). Every clip of 19 s or more ran to the 3 s deadline and
  fell back to the raw transcript, so it paid +2–5 s and got nothing back.
  Added release→text per clip: **median +3.1 s, p90 +4.2 s**.
* With no budget (30 s), the LLM alone took 2.1 s (5.8 s clip), 3.1 s (11 s),
  4.3 s (19.5 s) and 6.7 s (24.4 s).
* The gain was small: Parakeet already punctuates and capitalises. The 1.5B model added
  a final period and one comma. It made no harmful edits on these clips (no hallucinated
  words, no change of meaning).
* RSS went from 1.65 GB to 3.41 GB (+1.76 GB).
* During the second bench, flow-server was SIGKILLed (00:01:57). The whole box was out of
  memory: swap had 0 MiB free, earlyoom was active and other agents' builds were running.
  At 2.7 GB, flow-server was the largest process. systemd restarted it, but the reload
  took 70 s under that memory pressure. Under the task's rule ("if enabling breaks the
  server, restore"), I restored `config.toml.bak-pre-llm`. ASR then checked healthy
  (`llm:false`, jfk 951 ms, load 5.4 s).

## Setup

* Bench: `crates/flow-contract-tests/examples/hybrid_latency.rs` with new flags
  `--remote-only --reps 3 --jsonl …`. This is the real Router → flow-remote → `/v1/stream`
  path. Audio is pushed in 100 ms frames at real-time pace, and the clock runs from
  `commit` to the result (release→text, the client's view).
  Session: `{"partial_interval_ms":1500,"segment_pause_ms":800,"llm_timeout_ms":3000,
  "cleanup_level":"light","raw_mode":false,"ring_capacity_samples":2080000}`.
  That is the Android shell's config with cleanup requested. The same session was used
  before and after, so the before run also asked for cleanup, but the server had no LLM.
* Clips: jfk.wav plus the 8 good clips of segmented run `20261023-155524`
  (`.cache/segmented-2026-10-23/20261023-155524/long_0NNs.wav`). The file names are not
  the durations; the audio column is. Run `20261023-152113` was not used because those
  clips are broken.
* Target: the user's service on 127.0.0.1:7313. Phone sessions could run at the same
  time; none overlapped in the journal.
* Box load average was 8–36 throughout (other agents' builds). Absolute numbers are
  "loaded box"; before and after were equally loaded (before 8.7→13.4, after 10.7→15.1).
* Raw rows: `tools/probes/results/server-llm-2026-10-23/{before,after3000,after30000}.jsonl`.
  Tables: `tools/probes/prototypes/server_llm_cost_2026_10_23.py before.jsonl after3000.jsonl [--texts]`.

## What enabling needs (the installed server did not have it)

1. **A flow-server built with `--features llm`.** `tools/server/install.sh` builds
   `--features parakeet,tls` only. With that build, `llm_path` set is a startup error
   ("llm_path set but built without the `llm` feature"). Build:
   `SHERPA_ONNX_LIB_DIR=… cargo build --release -p flow-server --no-default-features --features parakeet,tls,llm`
   (the feature only adds libloading; there is no llama.cpp in the binary).
2. **The `flow-llm` plugin `libflow_llm.so`** (llama.cpp, a separate cdylib because of
   the ggml symbol clash). Point `llm_lib` at it in `[model]`, or put it next to the
   binary (`~/.local/bin/`), or set `FLOW_LLM_LIB`. Building it on this box needs
   `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"`; without it,
   bindgen fails on `stdbool.h`. For this run I used the release `libflow_llm.so` from
   the integration target, built 2026-10-22 (flow-llm is unchanged since 2026-10-20,
   ABI 1, prefill symbols present), installed as `~/.local/lib/flow-server/libflow_llm.so`.
3. `[model] llm_path = "~/.local/share/flow-server/models/qwen2.5-1.5b-instruct-q4_k_m.gguf"`
   (sha256 `6a1a2eb6…9407e`, checked).

The `ready` log line does not print llm state. Check `GET /v1/health` (= `hello_ok`)
for `"llm":true`. The installed binary is now the llm-capable build; it behaves
identically without `llm_path`. The previous binary is at `~/.local/bin/flow-server.bak-pre-llm`,
and the LLM-on config at `~/.config/flow-server/config.toml.llm-on-2026-10-23`.
Re-enabling is a config copy plus a restart. Downtime was 22 s on the swap (Parakeet +
LLM load), and 51–70 s cold or under memory pressure.

## Which client setting makes the server clean up

The server runs the LLM when **both** of these hold: it has a cleaner loaded, and the
session's `SessionConfig` has `cleanup_level != none` and `raw_mode == false`.
`hello_ok.llm` is only reported (router status). No client gates on it.

* **Android:** Settings → AI cleanup → **"Clean up with local AI"** (pref `llm_enabled`).
  The level comes from the pref through `Settings.effectiveCleanup`: off sends `cleanup_level:none, raw_mode:true` and the
  server never cleans up. On sends the selected Strength (None is promoted to Light) and
  `raw_mode:false`. The checkbox only appears in builds with an on-device engine
  (`caps.local`: local/hybrid flavours). The remote-only flavour cannot request server
  cleanup. Turning it on also makes the phone try to load a local GGUF; with none
  imported it shows a toast, and local fallbacks stay raw. The budget is fixed at
  `ANDROID_LLM_TIMEOUT_MS = 3000`.
  The phone currently sends none (all server sessions in the journal show `llm_ms=0`).
  I did not change the phone.
* **Windows:** sends `cleanup_level` (default **Light**) and `raw_mode` (default off)
  whatever `llm_enabled` is (`prefs.rs session_config`). A default Windows client
  therefore already requests server cleanup. But its default `llm_timeout_ms = 1500` is
  below what even the 5.8 s clip needed (1.7–2.1 s), so nearly every Windows dictation
  would time out to raw and pay +1.5 s.

## Real-world baseline

The phone debug log (`dict.jsonl`, D21 schema) has **0 remote entries**. All 17 are
`route: local`; the last one is 16:16Z, before the server route was in use. Local
on-device (15 rows with timings, median 10 words):

| metric | median | p90 |
|---|---|---|
| audio_ms | 4200 | 10112 |
| asr_ms | 375 | 1430 |
| total_ms | 429 | 2147 |
| perceived_ms (release→text) | 655 | 2755 |

Server side, the phone's sessions in the flow-server journal (Tailscale peer, since
2026-10-22): 26 sessions, 21 with text, 5 empty.

| metric (server) | median | p90 |
|---|---|---|
| audio_ms | 5240 | 18040 |
| asr_ms | 855 | 2438 |
| wait_ms (segment/partial in flight) | 254 | 2005 |
| total_ms | 1318 | 3396 |

Typical real dictations are 5 s (median) to 18 s (p90). At 3 s budget the bench shows
cleanup finishing at 5.8 s and 11 s and timing out from 19.5 s, so roughly the p75–p90
real dictations would pay the full +3 s for nothing.

## Bench: before / after per clip (medians of 3 reps, ms)

`llm_timeout_ms = 3000` (Android):

| clip | audio s | before asr | before e2e | after asr | after llm | after e2e | added | after status |
|---|---|---|---|---|---|---|---|---|
| long_005s | 5.8 | 1017 | 1022 | 738 | 1840 | 2580 | +1558 | 3× FORMATTED |
| jfk | 11.0 | 1051 | 1058 | 1032 | 2637 | 4069 | +3011 | 3× FORMATTED |
| long_015s | 19.5 | 1171 | 1265 | 1462 | 3002 | 6218 | +4953 | 3× RAW (llm_timeout) |
| long_010s | 24.4 | 2946 | 4405 | 2774 | 3002 | 7158 | +2753 | 3× RAW (llm_timeout) |
| long_028s | 29.9 | 2630 | 4133 | 2038 | 3000 | 8107 | +3974 | 3× RAW (llm_timeout) |
| long_025s | 33.9 | 1616 | 2033 | 1294 | 3001 | 5233 | +3200 | 3× RAW (llm_timeout) |
| long_020s | 34.3 | 2417 | 3409 | 1428 | 3000 | 5466 | +2057 | 3× RAW (llm_timeout) |
| long_040s | 41.7 | 1292 | 1481 | 1304 | 3003 | 4610 | +3129 | 3× RAW (2 timeout, 1 "llm deadline") |
| long_032s | 44.5 | 2297 | 3221 | 2247 | 3000 | 6836 | +3615 | 3× RAW (llm_timeout) |

All 27 runs: release→text **before median 2267 / p90 4262 → after median 5309 / p90 7538**.
**Added per clip: median +3129 ms, p90 +4170 ms.** `llm_ms` median 3000 (the deadline).
Release→text can exceed asr + llm. Prefill-ahead (`pre_asr_ms` up to 15 s on the 41.7 s
clip) and the segment decode in flight at release (`wait_ms`) take the single worker's
CPU during speech. The before run had to wait less for that.

`llm_timeout_ms = 30000` (1 rep; the run stopped after 4 clips when the server was killed):

| clip | audio s | llm_ms | after e2e | added vs before |
|---|---|---|---|---|
| long_005s | 5.8 | 2119 | 2928 | +1906 |
| jfk | 11.0 | 3110 | 4984 | +3926 |
| long_015s | 19.5 | 4303 | 5226 | +3961 |
| long_010s | 24.4 | 6732 | 9058 | +4653 |

This sits at the upper end of the earlier 1.1–5 s CPU estimate. LLM time grows roughly
linearly with transcript length (~65–75 ms per output word under this load).

## RAM

| state | VmRSS |
|---|---|
| no LLM, after one session | 1.65 GB |
| LLM loaded, idle | 3.41 GB |
| cgroup peak while benching with LLM | 2.7 GB (+swap 0) |

**+1.76 GB resident.** The box (20 GB RAM, 8 GB swap) was already at swap 7.9/8 GB from
other work before the change. The extra 1.8 GB made flow-server the largest process. It
was SIGKILLed at 00:01:57 during a memory-exhaustion episode (earlyoom also killed java
and node processes at 00:02:08).

## Quality (asr_text → final_text)

| clip | LLM change |
|---|---|
| jfk | none visible vs ASR. The raw route's deterministic post-processing splits "…for you. Ask what…"; the LLM route keeps the ASR comma "…for you, ask what…". Both fine. |
| long_005s | added the final period |
| long_015s | identical |
| long_010s | one comma ("companion, not one"), final period |

No hallucination, no dropped or added words, no change of meaning. It also fixed nothing
that matters ("to day", missing quote/dialogue punctuation stayed as they were). Parakeet
already emits punctuation and casing, so Light cleanup has little to do on clean read
speech. It would matter more on fillers and "scratch that", but the deterministic spoken
cleanup (`spoken_cleanup`, on by default) already handles those before the LLM.

## Verdict / next steps

* Keep server LLM **off** on this box: +3 s median for at most a period, and it destabilises
  a memory-starved host.
* If wanted later: (a) re-test on an idle box. Under this load the numbers are
  pessimistic, but the 3 s Android budget is still too tight for ≥ 15 s dictations.
  (b) Try the 0.5B model (`models/qwen2.5-0.5b-instruct-q4_k_m.gguf`; about half the
  RAM and time). (c) Raise the Android budget only for the server route (the router's
  commit timeout already adds `llm_timeout_ms`). (d) Skip the LLM above N words
  server-side instead of paying the full deadline.
