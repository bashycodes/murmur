# 2026-11-26: Screen names via a patched sherpa decoder (option A)

Research only: nothing here ships. Follow-up to
[2026-11-24-screen-names.md](2026-11-24-screen-names.md), sections "Why
Parakeet stops at 40 %" and "A higher Parakeet base score (rejected)".

**Question.** Screen-name hotwords stop at 40 % names on Parakeet. The cause
is in sherpa-onnx 1.13.8's NeMo modified beam search: before it picks the
top-k tokens, it adds the recognizer-wide `hotwords_score` to every token
that continues a hotword, whatever that phrase's own score. If the decoder
used the phrase's own score at that step, would screen names reach 50 %
within the gates?

**Answer: not as asked, but yes with one more change.**
- **A, as asked** (pre-pick bonus = the phrase's own score): names reach
  50 % at a screen score of 2.5, but the bench WER rises by +0.10 to
  +1.25 pt. The bonus stays in the hypothesis score in 1.13.8, so an
  unrelated screen now pushes the beam 2.5 instead of 1.0 on every clip.
  **Rejected.**
- **A2** (the phrase's own score, used *only to pick* the top-k tokens and
  not kept in the score, as upstream v2.0.2 already does with the global
  score): names reach **50-65 %** and the bench WER is unchanged (1.05 %)
  in every input. It costs one dictionary clip at base 1.0 (35 % → 30 %);
  with base 1.5, dictionary names rise to 45 %. **A2 at base 1.5 and
  screen 4.0 passes every gate.**

## Setup

* Probe: `tools/probes/prototypes/screen_names_2026_11_24/`, same set as
  before (20 espeak-ng name clips, 40 LibriSpeech bench clips). Summary rows
  have been appended to `summary-2026-11-24.jsonl`, with a `note` naming the
  variant.
* The probe changes: `screen_names_e2e` now writes `<mode>_ms` (the decode
  wall time) and `score.py` prints the mean. `run.sh` takes
  `SCREEN_NAMES_LIB=<dir>`, which runs the built test binary with that
  directory first on `LD_LIBRARY_PATH`. (`cargo test` would put
  `SHERPA_ONNX_LIB_DIR` first.) The only change to `crates/` is in the
  probe's own test file. No shipped code was changed.
* sherpa-onnx 1.13.8 (`murmur-asr-sherpa` pins `=1.13.8`), built from source
  on Linux x64 as a shared library (no TTS, C API only). It uses the
  `libonnxruntime.so` of the prebuilt 1.13.8 archive. Building
  `sherpa-onnx-c-api` at `-j 2` took about 4 minutes. Three libraries were
  built from the same tree: unpatched, A and A2.
* The baseline is the same source tree, unpatched. It reproduces the shipped
  base-1.0 row of the rejected section exactly: candidates 40 %,
  dictionary 35 %, candidates_dict30 45 %, and a bench WER of 1.05 % in
  every input.
* Gates (as before): screen names ≥ 50 %; dictionary-name accuracy no worse
  than the baseline; bench WER up ≤ 0.1 pt with no dictionary, with
  `dict30` and with `candidates_dict30`.
* Hebrew was not run. Both patches change only the offline NeMo decoder.
  Hebrew uses the streaming zipformer (`online-transducer-modified-beam-search-decoder.cc`),
  which neither patch touches and which has no pre-pick step.

## The patches

Both patches change one function,
`sherpa-onnx/csrc/offline-transducer-modified-beam-search-nemo-decoder.cc`.
The upstream file has CRLF line endings, and so do the patches. Apply them
with `patch -p1`.

* **A**: `sherpa-1.13.8-nemo-prepick-phrase-score.patch`. It changes one
  line and adds comments: `token_logits[token_id] += hotwords_score_` becomes
  `+= pair.second->token_score`. The context graph already stores the
  phrase's score on each arc. A shared prefix keeps the larger of the
  scores. A phrase without a score gets the graph's default, which is the
  recognizer's `hotwords_score`. So **with no per-phrase scores, A changes
  nothing**.
* **A2**: `sherpa-1.13.8-nemo-prepick-phrase-score-selection-only.patch`.
  It is A, plus the bonus goes into a copy of the log-probs that is used
  only for `TopkIndex`. The hypothesis score keeps the un-boosted
  log-prob, and the context graph adds its own score as before. **A2
  changes behaviour even with no per-phrase scores**: the global bonus no
  longer stays in the score. This is the same change upstream made in
  v2.0.2 (see "Upstream").

## Results

Base = the recognizer's `hotwords_score` (`MURMUR_PARAKEET_HOTWORDS_SCORE`),
which is what dictionary words get. Screen = the screen names' score
(`MURMUR_SCREEN_HOTWORDS_SCORE`). Cells give names % / bench WER. A bold
bench WER is more than 0.1 pt above the baseline.

| variant | base | screen | candidates | candidates_full | dictionary | dict30: bench | candidates_dict30 |
|---|---|---|---|---|---|---|---|
| unpatched (baseline) | 1.0 | 2.5 | 40 % / 1.05 % | 40 % / 1.05 % | 35 % / 1.05 % | 1.05 % | 45 % / 1.05 % |
| A | 1.0 | 2.5 | 50 % / 1.15 % | 50 % / **1.47 %** | 35 % / 1.05 % | 1.05 % | 50 % / **2.30 %** |
| A | 1.0 | 3.0 | 55 % / **1.99 %** | 60 % / **3.87 %** | 35 % / 1.05 % | 1.05 % | 60 % / **3.25 %** |
| A | 1.0 | 4.0 | 55 % / **11.94 %** | 60 % / **23.14 %** | 35 % / 1.05 % | 1.05 % | 55 % / **14.14 %** |
| A2 | 1.0 | 2.5 | 50 % / 1.05 % | 50 % / 1.05 % | 30 % / 1.05 % | 1.05 % | 50 % / 1.05 % |
| A2 | 1.0 | 3.0 | 55 % / 1.05 % | 55 % / 1.05 % | 30 % / 1.05 % | 1.05 % | 55 % / 1.05 % |
| A2 | 1.0 | 4.0 | 65 % / 1.05 % | 60 % / 1.05 % | 30 % / 1.05 % | 1.05 % | 60 % / 1.05 % |
| **A2** | **1.5** | **4.0** | **65 % / 1.05 %** | **60 % / 1.05 %** | **45 % / 1.05 %** | **1.05 %** | **65 % / 1.05 %** |
| A2 | 2.0 | 4.0 | 65 % / 1.05 % | 60 % / 1.05 % | 50 % / 1.05 % | 1.05 % | 65 % / 1.05 % |

In every run, `none` scored 10 % names and 1.05 % bench WER.

Names WER for candidates: baseline 15.76 %; A2 at 1.0/2.5, 13.04 %; A2 at
1.5/4.0, 8.15 %. `bubbles` is no longer sent. Its bench WER was 1.26 % at
the baseline, 2.09 % for A at 2.5, 1.05 % for A2 at 2.5, and 1.05 % for A2
at 1.5/4.0.

Verdict against the gates:
- **A fails.** At 2.5, `candidates` alone is on the edge (+0.10 pt), but
  `candidates_full` is +0.42 pt and `candidates_dict30` is +1.25 pt. Higher
  screen scores make it much worse. The bench errors are the same kind as
  in the rejected base-score sweep: real words split into hotword pieces,
  and a capital letter moved onto a hotword's first piece.
- **A2 at base 1.0 is one clip short.** Names and bench pass at every
  screen score. Dictionary names drop from 35 % to 30 %: one of the 20 clips
  flips to wrong, and none flips to right. Without the bonus kept in the score,
  a dictionary word at 1.0 gets less help than before.
- **A2 at base 1.5 and screen 4.0 passes every gate.** Names are 65 %
  (candidates) and 60 % (full capture). Dictionary names rise from 35 % to
  45 %. The bench WER is 1.05 % in every input. Base 2.0 gives 50 %
  dictionary names, still with no bench cost.

Why A2 works where A did not: in 1.13.8, the pre-pick bonus that admits a
token also stays in that hypothesis's score, even when the phrase is never
completed. Context-graph bonuses are taken back for an unfinished phrase;
this one is not. A higher bonus is therefore a permanent push towards
hotword pieces, on any audio. With the bonus used only to pick, a hotword
piece gets a place in the beam, and it then has to win on acoustics plus
the context graph's score, which is taken back if the phrase does not
finish. A higher screen score only lets more candidates in. That is why the
bench stays flat even at 4.0.

Caveats:
- The probe is small: 20 TTS name clips and 40 bench clips. 65 % is 13/20,
  and one clip is 5 pt.
- The bench is clean read speech. The rejected sweep's bench damage only
  showed with a dictionary, and A2 at base 2.0 shows none. A larger or
  noisier bench (for example the replay harness) should confirm this before
  anything ships.

## Cost

**Latency.** Here is the mean decode time per clip over all inputs (8
decodes × 60 clips, 4 threads, on the shared box):

| run | all inputs | candidates | none (patch not exercised) |
|---|---|---|---|
| unpatched, 1.0/2.5 | 826 ms | 847 ms | 744 ms |
| A, 1.0/2.5 | 800 ms | 796 ms | 703 ms |
| A2, 1.0/2.5 | 776 ms | 786 ms | 685 ms |
| A2, 1.0/4.0 | 742 ms | 746 ms | 664 ms |
| A2, 1.5/4.0 | 668 ms | 689 ms | 593 ms |

`none` sends no hotwords, so no patch code runs, yet it varies from 593 to
779 ms between runs. That is the box's noise, about ±15 %. No patched run
is slower than the unpatched one beyond that noise. The work the patches
add is one load per graph arc (A), plus a copy of 1025 floats per
expansion (A2). Both are negligible next to the joiner call. The one slow
outlier is A at 4.0 (875 ms), because its broken output emits more tokens.

**Size.** The patch is 1 line of code for A and 4 lines for A2, plus
comments. The patched `libsherpa-onnx-c-api.so` is the same size as the
unpatched one built here (3.4 MB, against 3.6 MB for the release
prebuilt).

## Upstream

The checks were read-only: nothing was opened or posted.
- **v2.0.2 (2026-11-19) already makes the pre-pick bonus selection-only.**
  PR #4084 ("Fix modified_beam_search dropping speech for NeMo TDT
  models", merged 2026-10-27) rewrote this decoder. Hypotheses are kept per
  frame, and (token, duration) pairs are explored. A new `length_reward`
  option is added. The hotword boost now goes into a `selection_logits`
  copy, "used for the selection only". It still adds the **global**
  `hotwords_score_`, not the arc's score. Upstream thus has half of A2.
  The other half, the per-phrase score at the pick, is one line on top of
  master.
- No upstream issue or PR asks for the per-phrase score at the pick. The
  closest open work: #4278 limits the hotword bonus to one token per frame
  in the NeMo beam searches (open, no maintainer review yet). #4276 and
  #4193 change how prefixes and unfinished matches are handled.
- **Can it go upstream?** The one-line change (`hotwords_score_` →
  `pair.second->token_score` in the `selection_logits` loop) changes
  nothing when no phrase has its own score, because every arc then carries
  `hotwords_score`. With per-phrase scores, it does what the hotwords
  docs suggest `:score` means. It is a small, self-contained PR against
  master, but it targets the post-#4084 decoder, not the 1.13.8 one.
- 2.0.x also changes greedy-versus-beam results (#4084 reports a lower WER
  for parakeet-tdt-0.6b-v2 with `length_reward` 0.5). So moving to 2.0.x
  would need its own bench run, whatever happens with this patch.

## What shipping would take

The A2 numbers above come from 1.13.8 plus a patch. Two routes:

1. **Fork 1.13.8 (A2 patch) and build sherpa ourselves on every platform.**
   - Android: already built from source (`build-sherpa-android.sh`, cached
     in CI by the script's hash). Apply the patch after the clone; this is a
     small change.
   - Server (Docker) and Linux: the Dockerfile downloads the
     `linux-x64-shared-no-tts` prebuilt (pinned sha256). It would need a
     CMake build stage, about 4 minutes at `-j 2`, or a patched library
     staged in `deploy/docker/prebuilt/lib`, which the Dockerfile already
     accepts.
   - Windows: CI downloads the `win-x64-shared-MT-Release-no-tts` prebuilt.
     It would need an MSVC CMake build of sherpa in CI, cached by version
     and patch hash. This is new work, and the largest piece.
   - We would carry the patch on every sherpa bump, and the 1.13.8 decoder
     is already gone upstream.
2. **Upgrade to 2.0.x and upstream the one-liner.**
   - 2.0.2+ already has selection-only boosting. It still uses the global
     score at the pick, so only the per-phrase part would be missing.
   - Until it merges, 2.0.x plus the one-line patch is the same fork, with
     the same Windows and server cost. But it rebases cleanly and goes away
     once upstream takes it.
   - Without the patch, 2.0.x would also allow the rejected base-score
     route again: the base bonus no longer stays in the score. Base 1.5-2.0
     might then reach 50 % with no fork. Untested here.
   - An upgrade needs its own bench and latency run (new search, new
     `length_reward`). The prebuilt archives' names and the
     `sherpa-onnx-sys` crate would change too.

The core side is small either way. For the A2 at base 1.5 and screen 4.0
config, `DEFAULT_HOTWORDS_SCORE` goes from 1.0 to 1.5 and
`SCREEN_HOTWORDS_SCORE` from 2.5 to 4.0. Both are now safe only with
selection-only boosting, so they must not ship on an unpatched 1.13.8.

## Recommendation

Do not ship a 1.13.8 fork for this; A, as asked, fails the gates. A2
shows the target is reachable (65 % names, no bench cost), and upstream
has already moved halfway there. If screen names come back, the cheapest
path is:
1. bump to sherpa 2.0.x on its own merits, with its own bench run;
2. re-run this probe on 2.0.x with a higher base, and with the one-line
   per-phrase patch;
3. offer that one-liner upstream, so no fork is needed.
