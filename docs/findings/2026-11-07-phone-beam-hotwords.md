# 2026-11-07: the phone decodes with beam + dictionary words when the dictionary has entries (D68)

**Question.** The PC decodes Parakeet with modified beam search and the
dictionary as sherpa hotwords; the phone was pinned to greedy (D22), so its
dictionary never reached speech recognition. Does beam + hotwords on the
phone's model reach 30/37 terms on the parity English set, at ≤ 150 ms added
p50, with an empty dictionary still greedy? Separately: learn names from a
History correction.

**Setup.** Probes `tools/probes/prototypes/phone_beam_2026_11_07/`
(scripts only; clips, dictionary and transcripts stay private).
Parity English set (29 clips, 37 terms, refs from the 2026-11-05 Wispr
capture), the phone's model (`sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8`),
the parity dictionary and the phone's session config. `flow-cli transcribe`
runs the full dictation pipeline (VAD, guard, rules, no LLM) on the
i7-6700 dev box, 4 threads. Terms and WER from the parity scorer. Beam
settings are the PC's (D12): `max_active_paths` 4, hotword score 1.0.

## Results (desktop)

| Path | Terms | WER | ASR p50 |
|---|---|---|---|
| greedy (today's phone) | 27/37 | 7.6 % | 496 ms |
| greedy + dictionary | 27/37 | 7.6 % | 471 ms |
| **beam 4 + dictionary (D68)** | **30/37** | **6.2 %** | 668 ms |
| beam 3 + dictionary | 30/37 | 6.2 % | 647 ms |

- Greedy + dictionary gives the same text as greedy on every clip: greedy
  never reads hotwords, so a dictionary on greedy does nothing. This is
  the empty-dictionary baseline too.
- Added latency, paired per clip, median: **+164 ms** (beam 4), +178 ms
  (beam 3). A narrower beam does not save time; the cost is the beam
  search plus the context graph, not the width. Engine-only sweep
  (`beam_sweep.sh`): greedy 26/37, beam 4 29, beam 3 29, beam 2 27,
  beam 1 27 (p50 496 / 645 / 613 / 618 / 499 ms). Beam 4 stays.
- **Latency criterion is not met on the desktop** (+164 vs ≤ 150 ms). The
  phone has not been measured; a phone-queue item asks for it. If the phone
  is over the budget too, the fix is in the beam search, not the width.

## Design

- **Which decoding.** `EnginePlan.parakeetDecoding(override, dictionary)`:
  a Developer override wins; otherwise BEAM when the dictionary has an
  entry that becomes a hotword (a word or a replacement, not a snippet,
  not blank), else GREEDY. Beam mode loads one recognizer, so RAM is the
  same as greedy (AUTO would load two, ~+0.6 GB). Hebrew Parakeet stays
  greedy (no Hebrew hotword measurement).
- **No live switch.** sherpa's transducer recognizer has no `SetConfig`
  (only the Whisper and Canary impls export one), so greedy ↔ beam means
  a new recognizer: a speech-model reload. `EngineHolder` keeps the
  decoding it loaded and reloads when the wanted one differs, at dictation
  start and when the dictionary changes (`DictionaryStore.onChange`). It
  happens only on the empty ↔ non-empty edge, and keeps the on-device LLM
  (`keepLlmForSwitch`). Desktop load ≈ 3.1 s either way; the phone's
  reload is unmeasured (Parakeet-he loads in ~4.7 s there).
- **Developer settings.** "Parakeet decoding" now defaults to "By
  dictionary"; Auto / Greedy / Beam remain as overrides.
- **Probe knobs.** `FLOW_PARAKEET_BEAM` (beam width) next to
  `FLOW_PARAKEET_HOTWORDS_SCORE`; `flow-cli --parakeet-decoding`.

## Name learning

- `learn_edit` flags a candidate as a name (`Candidate.name`,
  `learn::looks_like_name`) when every corrected word starts upper-case and
  has a lower-case letter (so not "I" or an acronym), and it is not just a
  single sentence-initial word ("Weather" → "Whether" is not a name).
- History's detail page gains **Correct the text**: the edit becomes the
  row's next version (as Retry does, D35), runs `learn_edit` through the
  core, and adds every candidate to the Dictionary's suggestions as before.
  A name that is not in the dictionary and not dismissed is offered at once
  with the suggestion dialog (Add word / Add as fix / Not now); Not now
  leaves it in the suggestions.
- Limits: learning needs the engine Ready (the core runs `learn_edit`);
  Windows has no History correction yet.

## Verification

- Rust: `learn` tests (names, sentence case, acronyms, mixed case, old
  JSON without `name`), flow-core lib tests, clippy `-D warnings`.
- Android unit tests: `ParakeetDecodingTest` (empty / snippet / blank →
  greedy, word or replacement → beam, override wins, Hebrew greedy),
  `EngineHolderTest` (a decoding change reloads once and keeps the LLM),
  `HistoryCorrectionTest`, `RetryStoreTest` (a correction is the next
  version), `DevSettingsTest` (override unset by default, persists, clears).
