# whisper_joins (2026-11-07)

Probe for the ticket "Parity · Whisper repeats text at segment joins".
Findings: `docs/findings/2026-11-07-whisper-segment-joins.md` (D66).

Scripts only. The parity clips, their references and every output are the
owner's and stay outside the repo; all paths are arguments.

| File | What |
|---|---|
| `src/bin/joins.rs` | `joins core`: real `FlowCore` commit per clip, whole vs `segment_pause_ms` 800 (unpaced). `joins pieces`: the D22 segmenter plus whisper directly, each piece decoded with and without `prior_text`; `PIECE_MIN_MS` zero-pads pieces (encoder-window floor test) |
| `run.sh` | `run.sh core\|pieces MODEL SESSION.json CLIPS_DIR OUT.jsonl [fit_ctx] [clip ...]`, low priority |
| `bench.sh AUDIO_DIR MODELS_DIR MODELS OUT.jsonl` | D20 40-clip bench (`tests/audio_ctx_bench.rs`), fitted window only; prints ASR total and `redecodes()` per model (D66 false-alarm check) |
| `wer.py` | `wer.py SCORE_PY REFS.json OUT.jsonl`: WER per clip and mode (the parity eval's `score.py` normaliser), repeated 4-grams across joins; prints numbers only |

Build: `CARGO_TARGET_DIR=... cargo build --release --offline -j 2` in this
directory (own workspace, flow-core features `whisper`, `wav`).
