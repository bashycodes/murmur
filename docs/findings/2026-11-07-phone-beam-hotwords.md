# 2026-11-07: the phone decodes with beam + dictionary words when the dictionary has entries (D68)

**Question.** The PC decodes Parakeet with modified beam search and the
dictionary as sherpa hotwords; the phone was pinned to greedy (D22), so its
dictionary never reached speech recognition. Does beam + hotwords on the
phone's model reach 30/37 terms on the parity English set, at ≤ 150 ms added
p50, with an empty dictionary still greedy? Separately: learn names from a
History correction.

**Setup.** Probes `tools/probes/prototypes/phone_beam_2026_11_07/`
(scripts only; clips, dictionary and transcripts stay private).
Parity English set (29 clips, 37 terms, refs from the 2026-11-05 reference
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

## No greedy fallback on the phone (beam only)

The PC decodes `AUTO`: beam 4 + hotwords, and a beam result that is empty
or implausibly short on VAD speech (< 0.5 words per second of speech) is
re-decoded by a second, greedy recognizer (the known beam-4 empty-result
bug). The phone's `BEAM` mode loads only the beam recognizer
(`greedy_fallback = false`), so it has **no greedy re-decode**. Measured to
decide:

- **RAM** (`ram_modes.sh`, peak RSS of one `flow-cli transcribe` on the
  phone's model, desktop): greedy 827 MB, beam 830 MB, **auto 1538 MB
  (+708 MB)**. The phone also holds the 1.5B cleanup model (D22 chose greedy
  to save exactly this second recognizer).
- **How often the fallback would fire** on the 30-line set with beam +
  dictionary: **0 empty results, 0 implausibly short** (lowest rate 1.47
  words per second of audio, threshold 0.5 per second of speech).

So `BEAM` stays: +0.7 GB for a re-decode that never fired on this set is
not worth it on the phone. Risk accepted: a dictation where beam returns
nothing or a fragment is not rescued by greedy. Two guards remain: clips
over 20 s are always decoded in ≤ 20 s pieces (D22, where the empty beam
results were seen), and an empty result on a shorter clip with speech is
still re-decoded in halves (beam, `needs_fallback`). If field logs show
empty beam results, the fix is `AUTO` on phones with RAM to spare, or a
greedy re-decode through a reload.

## Design

- **Which decoding.** `EnginePlan.parakeetDecoding(override, dictionary)`:
  a Developer override wins; otherwise BEAM when the dictionary has an
  entry that becomes a hotword (a word or a replacement, not a snippet,
  not blank), else GREEDY. Beam mode loads one recognizer, so RAM is the
  same as greedy (AUTO would load two: +0.7 GB, above). Hebrew Parakeet
  and call transcription stay greedy (`EnginePlan.fixedDecoding`; neither
  is measured with hotwords).
- **No live switch.** sherpa's transducer recognizer has no `SetConfig`
  (only the Whisper and Canary impls export one), so greedy ↔ beam means
  a new recognizer: a speech-model reload. `EngineHolder` keeps the
  decoding it loaded and reloads when the wanted one differs. A dictionary
  save (`DictionaryStore.onChange` → `EngineHolder.dictionaryChanged`)
  never switches on the UI thread or under a dictation, whose core the
  switch frees: it waits off the main thread until no dictation records or
  processes (`FlowApp.dictationBusy`); a dictation started first applies it
  at its start, before recording. It happens only on the empty ↔ non-empty
  edge, and keeps the on-device LLM (`keepLlmForSwitch`). Desktop load ≈ 3.1 s either way; the phone's
  reload is unmeasured (Parakeet-he loads in ~4.7 s there).
- **Developer settings.** "Parakeet decoding" now defaults to "By
  dictionary"; Auto / Greedy / Beam remain as overrides.
- **Probe knobs.** `FLOW_PARAKEET_BEAM` (beam width) next to
  `FLOW_PARAKEET_HOTWORDS_SCORE`; `flow-cli --parakeet-decoding`.

## Name learning

- `learn_edit` flags a candidate as a name (`Candidate.looks_like_name`,
  from `learn::looks_like_name`) when every corrected word starts upper-case and
  has a lower-case letter (so not "I" or an acronym), and it is not just a
  single sentence-initial word ("Weather" → "Whether" is not a name).
- History's detail page gains **Correct the text**: the edit becomes the
  row's next version (as Retry does, D35), runs `learn_edit` through the
  core, and adds every candidate to the Dictionary's suggestions as before.
  A name that is not in the dictionary and not dismissed is offered at once
  (Add word / Add as fix / Not now); Not now and Back leave it pending in
  the Dictionary's suggestions (only the Dictionary's own dialog has
  Dismiss, which hides a word for good). A manual correction does not
  change the Insights word counts.
- Limits: learning needs the engine Ready (the core runs `learn_edit`);
  without it the correction is still saved and a toast says its words were
  not learned. A failed save says so (not "Nothing changed"). Windows has
  no History correction yet.

## Verification

- Rust: `learn` tests (names, sentence case, acronyms, mixed case, old
  JSON without `name`), flow-core lib tests, clippy `-D warnings`.
- Android unit tests: `ParakeetDecodingTest` (empty / snippet / blank →
  greedy, word or replacement → beam, override wins, Hebrew greedy),
  `EngineHolderTest` (a decoding change reloads once and keeps the LLM; a
  dictionary change waits for the running dictation to end),
  `HistoryCorrectionTest`, `HistoryUiTest` (correction without a loaded
  engine says so; a failed save is reported), `RetryStoreTest` (a
  correction is the next version), `DevSettingsTest` (override unset by
  default, persists, clears).
