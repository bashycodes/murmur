# 2026-11-24: Auto-detect between Hebrew and English (D106)

**Question.** The owner reports that, on 1.3.0, auto-detect on Windows mixes up
Hebrew, English and mixed dictations. How does auto-detect work, how often does it
pick wrong, and can it be fixed without hurting English?

**Answer.** Language ID decided from the **first 3 s of speech only** (D34). Every
dictation that changes language after 3 s stayed in the first language. A Hebrew
dictation whose first 3 s sounded English (a breath, a pause, a name) went to the
English model as a whole. The fix keeps identifying each further 3 s of speech while
recording. At commit, the dictation goes to the language that **most of its speech**
is in. On the probe set, routing accuracy goes from **86.4 % to 93.2 %**:
mixed dictations 41/80 → 64/80, Hebrew 108/120 → 112/120, English unchanged
(156/156). English WER is unchanged at **1.60 %** (identical text on every English
clip), and Hebrew WER goes from 31.3 % to 27.8 %.

## How auto-detect works (Windows 1.3.0 and today)

* **Which dictations are auto-detect.** Settings → General → Language
  "Detect automatically" means `prefs.languages = []` and
  `keyboard_language = false`. The default, "Keyboard language" (D37), is **not**
  auto-detect: an English or Hebrew layout *forces* that language, and the spoken
  language is never checked. Only a layout the table does not know falls back to
  auto.
* **When language ID runs.** Only when a Hebrew model is chosen
  (`prefs.hebrew_model`) **and** `ggml-tiny-q5_1.bin` is in the models folder
  (`LanguageSpec.lid_path`). Without the file, auto-detect is always English, with
  no warning. The file comes with the Hebrew downloads, but an import may leave it
  out (it is optional there). The server route behaves the same way: the server
  runs the same core, with `MURMUR_LID` / `lid_path`, so the server route
  gets D106 only after murmur-server is upgraded. A shell on the new version
  that dictates through an old server still gets the first-window rule.
* **The decision (D34).** `engine::identify_language`: once 3 s of speech are in
  (from the energy VAD's speech start), whisper tiny scores {en, he}. The session
  takes the Hebrew route when p(he) ≥ 0.1 (`lid::ROUTE_MIN_PROB`). Otherwise it
  uses the default model (Parakeet English). Dictations shorter than 3 s are
  identified at commit. Nothing looked at the rest of the dictation.

## Where it goes wrong (measured, before)

1. **Language switches after 3 s.** A dictation that starts in English and
   continues in Hebrew (or the other way round) is decoded entirely in its first
   language. Mixed sets: **23/40** and **18/40**, where the language with more speech
   counts as right. In practice that is a coin flip on which language came first.
2. **Hebrew that starts "English-sounding".** 10 of 120 Hebrew clips scored
   p(he) < 0.1 on their first window, but ≥ 0.1 on most later windows (one clip:
   0.001, 1.0, 0.998). All 10 went to Parakeet English, which writes Hebrew speech as
   English-looking nonsense.
3. **Not a fault.** English never went to Hebrew: 0/156 English and short-English
   clips, and the highest p(he) on any English window was 0.087. Short (2 s)
   dictations are as good as long ones (he 37/40, en 39/39). Two FLEURS clips are
   too quiet for the energy VAD (`no_speech`). That is a known FLEURS quirk
   (2026-10-30 finding), not language ID.
4. **Not measured.** Accented English, Hebrew with single English words inside a
   sentence, and the cloud route (the provider detects the language itself).

## Probe

`tools/probes/prototypes/lang_detect_2026_11_24/` holds `prepare.py`, `run.sh` and
`score.py`. The core test is `crates/murmur-core/tests/lang_detect_e2e.rs`. Public
audio only, and the audio is never committed:

| set | n | what |
|---|---|---|
| he | 120 | FLEURS he_il dev (every 2nd clip of dev.tsv) |
| en | 117 | LibriSpeech test-clean: the 40-clip bench set + ≤ 2 more per speaker |
| he_short / en_short | 40 / 39 | the first 2.0 s of speech of every 3rd clip |
| mix_en_he / mix_he_en | 40 / 40 | an English and a Hebrew clip, 0.4 s apart, level-matched; `expect` = the language with more speech |

Every clip starts with 0.3 s of near-silence (the key press). There are two modes:

* `run.sh route` runs each clip through `MurmurCore`: auto-detect, the Hebrew
  route, whisper tiny-q5_1 LID and D22 segmentation at 800 ms. It uses mock
  decoders by default. With `EN_ASR` / `HE_ASR` it uses real ones (Parakeet TDT
  0.6B v2 int8 and zipformer2-66m he), which gives WER.
* `run.sh scores` dumps the identifier's en/he scores for each window. It uses
  the engine's own windowing (`lid::first_window` and `lid::window_after`), but
  scores every window, without the engine's early stop. `score.py rules` then
  compares routing rules offline.

The machine was the shared i7-6700, with 4 threads.

**Reproducing.**

1. Run `prepare.py` once (it builds the manifest under `$LANG_DETECT_DATA`).
2. For the **before** rows, check out commit `fdba6239` on this branch. That is
   the probe alone, on top of `main`, with the D34 engine unchanged. Run
   `LID_MODEL=<ggml-tiny-q5_1.bin> run.sh route` there, once with mock decoders
   and once with `EN_ASR`/`HE_ASR`. The runs here were made before a rebase, on
   the identical tree, apart from one doc comment in `config.rs`.
3. For the **after** rows, run the same commands on the branch head.
4. The per-set counts, WER and release latency of every run quoted here are
   committed. `summary-2026-11-24.jsonl` comes from `score.py summary`. The run
   called `final-real` is the "tie keeps the current route" variant below.
   `rules-2026-11-24.txt` holds `score.py rules` for tiny and base.

**Two tables, two denominators.**

* The rules table below is offline. It counts only clips where the VAD finds
  speech: he 119/120 and he_short 39/40. One FLEURS clip, and its short cut, are
  too quiet for the energy VAD.
* The end-to-end tables count every clip. A commit that returns `no_speech`
  counts as wrong. That happens for two Hebrew clips (the quiet one, plus one
  that the commit-time check also drops) and for their two short cuts.

So the D34 rule shows 109/119 offline and 108/120 end to end. The 10 Hebrew
clips it sends to English are the same 10 in both.

## Rules compared (per-window scores, tiny-q5_1; base-q5_1 in brackets)

| rule | he /119 | en /117 | he_short /39 | en_short /39 | mix en→he /40 | mix he→en /40 | total |
|---|---|---|---|---|---|---|---|
| D34: first window, p(he) ≥ 0.1 | 109 (109) | 117 | 38 | 39 | 23 (22) | 18 (19) | 87.3 % (87.3) |
| first window, argmax | 94 (101) | 117 | 32 (36) | 39 | 22 | 16 (20) | 81.2 % (85.0) |
| mean over all windows ≥ 0.1 | 118 (119) | 117 | 38 | 39 | 18 (19) | 19 (20) | 88.6 % (89.3) |
| **speech share: windows vote (p ≥ 0.1), most speech wins** | 113 (117) | 117 | 38 | 39 | 35 (34) | 29 (27) | **94.2 %** (94.4) |
| clipped log-odds sum from the 0.1 floor, ±3 | 115 (118) | 117 | 38 | 39 | 32 (35) | 31 (28) | 94.4 % (95.2) |
| clipped log-odds sum from 0.05, ±3 | 117 (119) | 117 | 38 | 39 | 33 (35) | 31 (28) | 95.2 % (95.4) |

The averages win on pure Hebrew but lose mixed dictations, because one
confident window outweighs the rest. The speech-share vote is the simplest rule
that fixes mixed dictations. Its window rule is unchanged from D34 (p ≥ 0.1), so
a single window, such as any dictation under 3 s, decides exactly as before.

The log-odds variants score up to 1 point higher, within this set's noise. The
0.05 variant lowers the per-window floor, which the 0.087 English window argues
against.

Base-q5_1 adds 4 Hebrew clips, at about twice the cost per window.

## Fix (D106), end to end through MurmurCore

The fix is `lid::choose_whole` plus `engine::identify_while_recording` /
`identify_at_commit`.

* **While recording.** The first window still routes the session at 3 s, so D22
  segments are decoded while the user speaks. After that, each further 3 s of
  speech is scored while recording: about 70 ms each, on the worker, one per
  tick.
* **At commit.** Windows not yet scored are scored only while they could still
  change the answer (`lid::decided_without`). The loop checks the commit's
  cancel flag before each window. Then the session moves to the language with
  the most speech. A move re-decodes the pieces that were decoded on the other
  engine, because their requests no longer match.
* **Ties.** A tie goes to the routed language heard first. Keeping the current
  route on a tie measured worse: the `final-real` run gave he 110/120,
  mix en→he 33/40 and 92.2 % in total. Ties are common, because a 6 s mixed
  dictation is two 3 s windows.

| run (real decoders) | he /120 | en /117 | he_short /40 | en_short /39 | mix en→he | mix he→en | total | en WER | he WER |
|---|---|---|---|---|---|---|---|---|---|
| before (`fdba6239`) | 108 | 117 | 37 | 39 | 23 | 18 | 86.4 % | 1.60 % | 31.30 % |
| after (branch head) | **112** | 117 | 37 | 39 | **35** | **29** | **93.2 %** | **1.60 %** (0 texts changed) | **27.75 %** |

The mock-decoder runs give the same routing table.

* Two Hebrew clips that D34 had right now go to English. Their later windows
  score p(he) < 0.1.
* Six clips that D34 had wrong are now right.

**Release latency** is commit → text, median / p90 in ms. The probe pushes each
clip at once, so every window not yet decided is scored at release. That is the
worst case.

| set | before | after |
|---|---|---|
| en | 670 / 1090 | 745 / 1168 |
| he | 490 / 1042 | 567 / 1078 |
| he_short | 225 / 263 | 233 / 280 |
| en_short | 325 / 373 | 327 / 358 |
| mix en→he | 2398 / 3849 | 2082 / 3045 |
| mix he→en | 1381 / 2316 | 2328 / 3780 |

**`Timings::lid_ms` is not latency.** It sums every window, including those
scored while recording (mean 95 → 210 ms here). Release latency is `total_ms`,
the figure in the table above.

When recording in real time, the extra windows are scored while the user speaks.
For a one-language dictation, the windows left at release are skipped, because
they cannot outvote what came before
(`language_id.rs::one_language_throughout_…`). So the extra ≈ 75 ms above only
shows up when the audio arrives all at once (History retry, file replay).

A dictation that switches language pays one re-decode on the other model at
release. For Hebrew→English on Parakeet that is about +0.9 s median, the cost of
getting it right.

## Options not taken (follow-ups)

* **Base-q5_1 as the LID model.** It gets 117/119 Hebrew with the same rule (tiny
  gets 113), at about 155 ms per window instead of 70. That needs a new catalogue
  pin and download (32 → 60 MB) on Windows, Android and the server library.
* **A clipped log-odds rule** (`score.py rules`), up to +1 point on this set. Re-check it on owner-like
  audio before switching.
* **Warn when auto-detect has no LID file.** With a Hebrew model but no
  `ggml-tiny-q5_1.bin`, auto-detect is silently English-only. Settings could say so.
* **Within-sentence code-switching** (Hebrew with an English term) is not
  measured. Windows are 3 s, so one English word does not move a Hebrew dictation,
  which is what we want for the Hebrew model.
