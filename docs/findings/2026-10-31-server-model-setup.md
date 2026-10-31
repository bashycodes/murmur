# Home server model setup on 0.12.0 (2026-10-31)

**Question.** Which models should the home flow-server (i7-6700, 4C/8T AVX2,
no GPU, ~19.9 GB RAM shared with other jobs) serve, and does the 0.12.0 build
serve them end to end?

**Answer.** Deployed `flow-server 0.12.0+5a27023` (tag v0.12.0) with:

| slot | model | why |
|---|---|---|
| default (English) ASR | Parakeet-unified-en 0.6B int8 | best on this CPU: 1.15 % WER, no long-audio drops (D23, `docs/proposals/stt-model-survey.md`) |
| `[model.languages.he]` | zipformer2-66m-streaming-he int8 (`sherpa_online`) | 26.8 % FLEURS WER vs parakeet-he 28.8 %, RTF 0.05 (`2026-10-28-hebrew-stt-models.md`) |
| `lid_path` (auto-detect) | whisper ggml-tiny-q5_1 | 70 ms mean, 79/81 routed right (`2026-10-30-hebrew-language-id.md`) |
| AI cleanup (sessions + D36 `clean`) | Qwen2.5-1.5B-Instruct Q4_K_M | production pipeline 92 %, LLM p50 1.37 s; 3B p50 2.9 s is too slow here (`2026-10-26-ai-cleanup-benchmark.md`) |
| threads | 4 (ASR and LLM) | one per physical core; hyperthreads are slower on this box |

The previous install was 0.6.0 (Oct 27): no `clean` (D36), no per-language
models, no LID.

## Install

* Systemd **user** unit (install.sh `--user` layout): binary in
  `~/.local/bin`, `libflow_llm.so` + sherpa/onnxruntime libs in
  `~/.local/lib/flow-server`, models in `~/.local/share/flow-server/models`,
  config + token in `~/.config/flow-server`. Unit unchanged (MemoryMax 8G, Nice 5).
* Build: detached worktree at `v0.12.0`, `cargo build --release --offline -j 2
  -p flow-server --no-default-features --features whisper,parakeet,tls,llm`
  plus `-p flow-llm`, under the shared build lock (16.6 + 2.9 min).
  **`tools/server/install.sh` cannot build this config:** it builds
  `parakeet,tls[,llm]` without `whisper`, and `lid_path` needs the `whisper`
  feature (`flow_core::engine::load_lid` refuses otherwise). So the binary and
  plugin were installed by hand exactly where install.sh puts them; the
  sherpa libs were already the 1.13.8 ones (same hashes), and the config was
  edited (install.sh never edits an existing config).
* Models copied from the local caches; SHA-256 checked first: zipformer-he's
  four files against `tools/fetch-models.sh` pins, ggml-tiny-q5_1 against the
  Windows catalogue pin (`model_download.rs`; fetch-models.sh has no LID pin).
* `check-config` passed before the restart. Startup: `ready
  version="0.12.0+5a27023" workers=1 asr=parakeet languages=he=sherpa_online`,
  load 84 s (box swapping). RSS 3.1–3.7 GB (was 1.65 GB on 0.6.0, partly
  swapped out).
* `/v1/health`: `features: ["segments","clean"]`, `languages: ["he"]`, `llm: true`.

## Verification (real-time streamed WAVs, probe `tools/probes/prototypes/server_e2e_2026_10_31.py`)

Clips: `jfk.wav` (11 s, English) and FLEURS he_il dev clips (16 kHz float).
Session `llm_timeout_ms` 3000, cleanup medium. Times are commit → result.
The box was loaded (other agents' builds, load 6–13, swap ~95 % full).

| clip | languages | detected | text | asr ms | llm ms | commit→result ms |
|---|---|---|---|---|---|---|
| he 7.6 s | `he` | he | correct Hebrew | 393 | skipped | 396 |
| he 7.6 s | `en` | en | Latin gibberish (Parakeet-en on Hebrew) | 1474 | skipped | 1481 |
| he 7.6 s | auto | he | same Hebrew as forced `he` | 403 | skipped | 406 |
| he 6.5 s | `he` / auto | he / he | exact match to reference | 217 / 304 | skipped | 219 / 307 |
| he 13.3 s | `he` | he | Hebrew, all words; the speaker reads the first 3 words last (see notes) | 711 | 3000 (timeout) | 3714 |
| jfk 11 s | auto | en | correct, Formatted | 1234 | 2578 | 3816 |
| jfk 11 s | `en` | en | correct, Formatted | 919–1197 | 2412–2650 | 3335–3850 |

* **Routing confirmed:** forced `he` and auto-detected Hebrew decode on
  zipformer-he (Hebrew script, ~0.03–0.05 × audio); forcing `en` on the same
  clip gives Parakeet-en's Latin output; English auto-detect stays on Parakeet.
  LID adds ~10–90 ms.
* **D36 `clean`:** 5 requests, all cleaned text, `llm_ms` 1067–2159
  (e.g. "so um the the deploy is done can you check the logs" → "The deploy is
  done. Can you check the logs?"; a spoken list came back as a formatted list).
* First English dictation after start timed out its LLM (cold); later ones fit.

## Cleanup suite through the server (`server_clean_bench_2026_10_31.py`)

The 102 bench cases (88-suite + 14 Wispr), raw input, prompt v4c, one `clean`
each, 8 s timeout, scored with `ai_cleanup_bench_2026_10_26/score.py`. This is
**raw-in AI cleanup without guard or fallback rules** (the server returns the
model's answer; the client guards under D36), so it is comparable to the
"raw-in" rows, not to the 92 % production pipeline (rules → v2 → guard → skip
inside a session), which `clean` cannot exercise.

| run | 88-suite | target-closeness | LLM p50 / p90 |
|---|---|---|---|
| this box, server `clean`, v4c, 1.5B (loaded box) | **65 %** | 0.803 | 2252 / 4844 ms (5 timeouts) |
| 2026-10-26 raw-in v4b 1.5B (this box, local runner) | 69 % | 0.866–0.871 | ~1.5 s |
| laptop v4c raw-in 1.5B CPU t4 (2026-10-30) | 74 % | — | 1041 ms |

In line with the documented raw-in numbers given the load; v4c via the server
is not better than the in-session production pipeline on this model, which
supports keeping sessions on the production path. 4 min 35 s wall.

## Notes / follow-ups

* `tools/server/install.sh` should gain `whisper` in its feature list (or a
  `--lid` switch) so a re-install keeps LID working; today a plain
  `install.sh --user` re-install would build without `whisper` and the server
  would refuse to start with this config.
* zipformer-he seemed to drop the first ~3 words of the 13.3 s FLEURS clip
  both forced and auto. Not a bug: the speaker says those words near the
  end of the clip and the output has them there
  (`2026-10-31-zipformer-first-words.md`). One FLEURS clip (peak −46 dBFS) is VAD `no_speech`, expected.
* Hebrew LLM cleanup on long dictations exceeds a 3 s budget.

## Rollback

Timestamped copies `*.bak-20261031-1942` next to the binary, the plugin
`libflow_llm.so` and `config.toml`, and the user unit at
`~/.config/flow-server/flow-server.service.bak-20261031-1942`. Restore the
three files and `systemctl --user restart flow-server`. Sherpa libs and the
unit were not changed.
