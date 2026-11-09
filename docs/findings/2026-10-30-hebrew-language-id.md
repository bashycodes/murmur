# Hebrew auto-detect: spoken language ID (H6, D34) — 2026-10-30

Question: with language = auto, can murmur-core tell Hebrew from English before
decoding, cheaply enough (target ≤ 150 ms on the i7-6700) and offline on the
server, Windows and Android?

## Options

| option | runtime | extra model | encoder work | verdict |
|---|---|---|---|---|
| whisper.cpp `whisper_lang_auto_detect` (whisper-rs, already in core) | linked in every shell | multilingual ggml, 31 MB (tiny-q5_1) | window can shrink to the clip | **chosen** |
| sherpa-onnx spoken-language-identification (1.13.8) | only in `parakeet` builds | whisper tiny onnx, ~40 MB int8 | always 30 s | slower, and missing from Windows `local` builds |

whisper.cpp's language detection encodes with the window the state's last
`whisper_full` used, and has no setter for it. `WhisperLid::load` therefore
runs one 1-token decode of silence with `audio_ctx` = 4 s (3 s + 1 s), and
every later detection uses that window.

## Measurement

* Data, not committed: 40 FLEURS he_il **dev** clips (every 8th file) and 41
  English clips (40 LibriSpeech test-clean + JFK).
* Input: the first 2 s or 3 s after the energy VAD's speech start. Candidates
  {en, he}, renormalised.
* Machine: i7-6700, 4 threads, a shared box (load average ≈ 7).
* Reproduce with `crates/murmur-core/tests/lid_e2e.rs`; its header lists the env.

| model | window | head | latency mean / p90 / max | routed right (p(he) ≥ 0.1) | + 10 dB white noise |
|---|---|---|---|---|---|
| tiny-q5_1 | 4 s | 3 s | **70 / 81 / 143 ms** | **79/81** (he 38/40, en 41/41) | 70/81 |
| tiny-q5_1 | 4 s | 2 s | 69 / 90 / 151 ms | 76/81 | 62/81 |
| base-q5_1 | 4 s | 3 s | 155 / 216 / 247 ms | 79/81 | **75/81** |
| base-q5_1 | 4 s | 2 s | 156 / 216 / 253 ms | 77/81 | 75/81 |
| tiny (f16) | 30 s | 3 s | 779 / 878 / 932 ms | 75/81 (at p ≥ 0.5) | — |

* Whisper errs towards English. English clips never gave Hebrew more than
  0.02, and noise only made that lower. Hebrew clips often scored only
  0.1–0.5, and noise pushed them further down. So the core routes at
  p ≥ 0.1, not at the argmax:
  * with tiny at 3 s, the argmax gets 30/40 Hebrew clips right;
  * p ≥ 0.1 gets 38/40, still with no English clip routed wrong.
* Short heads hurt: 2 s is clearly worse than 3 s on tiny.
* Latency is mostly hidden. A dictation that is longer than 3 s is identified
  while the user is still speaking. Only shorter ones pay the ~70 ms at
  release.
* Untested: accented English (an Israeli speaker's English could score
  higher for Hebrew), and Hebrew–English code-switching inside the first 3 s.
* Side finding: the energy VAD reports `no_speech` on 3 of the first 8 FLEURS
  clips (quiet recordings). Those dictations are empty before language ID
  runs. Phone audio is louder, but keep it in mind if Hebrew tests on
  FLEURS look empty.

## Decision

`ModelConfig.lid_path` points at `ggml-tiny-q5_1.bin`, and at base-q5_1 where
noise matters and 150 ms is fine. Language ID runs only with that path set
and at least one `languages` entry. Wiring a download/setting for the model
into the Windows and Android shells (and server TOML docs) is follow-up work.
