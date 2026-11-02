# H7: the owner's Hebrew dictations, zipformer-he vs parakeet-he (2026-11-02)

**Question.** The Hebrew default (`docs/proposals/hebrew-stt.md` decision 2,
zipformer-he) was chosen on FLEURS read speech. Does it still win on the
owner's own dictation voice, on the host and live on the phone?

**Answer.** Yes. zipformer-he beats parakeet-he on every accuracy metric,
answers about 8× faster after the last audio and is a tenth of the size.
The phone's live zipformer-he matches the host run. **zipformer-he stays the
Hebrew default; parakeet-he stays selectable.**

## Setup

* **Clips:** 16 Hebrew dictations by the owner, read from a prepared list of
  sentences, 132 reference words (571 letters, 78 s of audio), recorded
  2026-11-02 on the phone (hybrid build `a494ac6`). Two English warm-up
  dictations from the same session are left out. The owner made no edits.
  The recordings and the reference sentences are personal and stay outside
  the repo; this note has aggregates and error categories only.
* **Host runs:** the warm-recognizer probe from the earlier Hebrew findings
  (`tools/probes/prototypes/stt_survey_2026_10_23`), sherpa-onnx 1.13.8,
  4 threads, greedy decoding, on a busy box (load ≈ 11, so RTFs are
  pessimistic).
  * zipformer-he (`zipformer2-66m-streaming-he` int8) as an online
    recognizer fed in 100 ms chunks, then 0.8 s of silence and
    `input_finished`; the **+1.5 s tail** row adds 0.7 s more silence, the
    tail the app uses (proposal decision 3).
  * parakeet-he (`parakeet-tdt-0.6b-v3-he` int8) as an offline
    `nemo_transducer`.
* **Phone live run:** the `sherpa_online` zipformer-he raw output the app
  logged for the same 16 dictations.
* **Normaliser:** as in `2026-10-28-hebrew-stt-models.md`: strip niqqud,
  geresh and punctuation, split on whitespace; final letters not folded,
  digits kept as written. CER is letter edit distance without spaces.
  "Numbers folded" counts a number written in digits as matching the
  spelled-out reference.

## Scores (16 clips, 132 words)

| model | WER | CER | WER, numbers folded | CER, numbers folded | RTF | latency after last audio | size |
|---|---|---|---|---|---|---|---|
| zipformer-he, host | 14.4 % (19/132) | 9.1 % | 9.1 % | 3.5 % | 0.054 | ≈ 50 ms | 72 MB |
| zipformer-he, host, +1.5 s tail | **13.6 %** (18/132) | **8.9 %** | **8.3 %** | **3.3 %** | 0.070 | ≈ 50 ms | 72 MB |
| parakeet-he, host | 18.9 % (25/132) | 11.2 % | 10.6 % | 4.2 % | 0.085 | ≈ 420 ms (whole decode) | 673 MB |
| zipformer-he, phone (live) | 14.4 % (19/132) | 9.3 % | 9.1 % | 3.7 % | ≈ 0.07 | — | 72 MB |

* One word is 0.76 WER points. With numbers folded the zipformer leads by
  2–3 words (6–7 raw), so the margin is directional, but it agrees with
  FLEURS (26.8 % vs 28.8 %, `2026-10-28-hebrew-stt-models.md`).
* Latency: zipformer-he has already decoded the audio while it streamed, so
  only the tail is left after the last audio; parakeet-he decodes the whole
  buffer then. zipformer-he also loads 2–3× faster (5–7 s vs 11–20 s).
* Phone ASR time per ≈ 4.6 s clip: 219–535 ms, median ≈ 340 ms.

## Phone vs host

* The phone's text equals the host zipformer's on **12 of 16** clips
  (against the +1.5 s tail run; 11 against the shorter tail). On one clip
  the shorter host tail cut the last letter off the sentence-final word;
  the phone and the 1.5 s tail run got it right, so **the app's 1.5 s tail
  helps** and should stay.
* The remaining differences are different garbles of the same words at
  chunk boundaries (the phone feeds the whole buffer at commit, the host fed
  100 ms chunks). Overall WER is identical; no systematic gap.

## Error categories

1. **Numbers rendered as digits** (the largest single source of counted
   errors). Both models write spoken numbers, and parakeet-he also times,
   in digits, always with the right value. Folding them drops WER by 5–8
   points for both models. It is a scoring artefact, not a usability
   problem; a future Hebrew ITN or reference normaliser should treat digits
   and number words as equal.
2. **Sentence-final and short rare words garble on both models**, usually
   on the same words. The audio is not truncated (silence follows each), so
   these are model errors, not capture errors.
3. **zipformer-he misses some English loanwords that parakeet-he gets.**
   Common loanwords were right on every model, but on the two harder ones
   parakeet-he was right and zipformer-he was not.
4. **parakeet-he garbles more native Hebrew morphology** (prefixed and
   inflected words) that zipformer-he gets right.
5. **No first-word drops** on any model or run (0 of 16). The 2026-10-31
   suspicion (`2026-10-31-zipformer-first-words.md`) stays cleared.

## Decision

Keep **zipformer-he as the Hebrew default** (proposal decision 2 holds on
the owner's voice): better WER and CER, ≈ 50 ms vs ≈ 420 ms after the last
audio, 72 MB vs 673 MB, and the phone matches the host. **parakeet-he stays
selectable** for dictation heavy in English loanwords. No code change.
