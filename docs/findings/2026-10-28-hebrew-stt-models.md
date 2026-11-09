# 2026-10-28: Hebrew STT models — top 3, CPU pick, no-limits pick

Question (ticket "Research Hebrew STT models"): which open Hebrew speech-to-text
models are the best today, which one should run on our CPU, and which one is the best
if compute does not matter?

Sources: the ivrit.ai Hebrew transcription leaderboard (its raw `benchmark.csv`, last
commit 2026-10-14), the model cards, and our own run on this box (marked
**measured**). Anything not measured is marked **estimate** and says what it is based
on. All WER numbers are percentages; lower is better.

## TL;DR

| # | model | params / size | Hebrew WER eval-d1 / eval-whatsapp / FLEURS (leaderboard) | licence | runtimes we already have | streaming | on our i7-6700 (4 threads) |
|---|---|---|---|---|---|---|---|
| 1 | **ivrit-ai/whisper-large-v3** (2025-05-13 checkpoint) | 1.55B; ggml f16 3.1 GB | **5.1 / 7.2 / 17.4** [1] | Apache-2.0 [3] | whisper.cpp (ggml [4]), CT2 | no (30 s window) | **estimate** ≥ 40 s per dictation: too slow |
| 2 | **ivrit-ai/whisper-large-v3-turbo** (2025-05-13) | 0.8B; ggml f16 1.6 GB | 5.3 / **7.1** / 18.1 [1] | Apache-2.0 [5] | whisper.cpp (ggml [6]), CT2, ONNX | no | **estimate** ≈ 25 s per clip (RTF 3.3); ≈ RTF 1 with `-ac` |
| 3 | **ivrit-ai/zipformer2-66m-streaming-he** (2026-10) | 66M; **72 MB int8** | 5.3 / 8.8 / 23.4 [1][7] | CC-BY-4.0 [7] | **sherpa-onnx** (`OnlineRecognizer`) | **yes** (1.28 s chunks) | **measured** RTF 0.05; **48 ms** from end of speech to text |
| (4) | thewh1teagle/parakeet-tdt-0.6b-v3-he, int8 export by krut42 | 0.6B; 673 MB int8 | 6.1 / 8.8 / ≈23 (card, not on the leaderboard) [8][9] | CC-BY-4.0 | **sherpa-onnx `nemo_transducer`: drop-in, zero code** | no | **measured** RTF 0.069, 0.57 s per 8 s clip |

| pick | model | why, in one line |
|---|---|---|
| **Best for our CPU** (murmur-server i7-6700; also the Windows PC and phone) | **zipformer2-66m-streaming-he int8** | Within 0.2 points of the best open model on conversational Hebrew, 20× faster than real time, streaming, 72 MB, and it runs on the sherpa-onnx stack we already ship. |
| **Best ignoring compute** | **ivrit-ai/whisper-large-v3** (turbo as the practical GPU variant) | Lowest WER of any open model on 4 of 5 leaderboard sets. It needs an NVIDIA GPU with ≈ 4.5 GB VRAM (fp16) or ≈ 3 GB (int8) [11]. Turbo is within 0.7 points (and 0.1 better on whatsapp) at roughly half the cost: 0.5–0.7 s per 5–15 s clip on a 2080 Ti or T4 [2]. |

Day-1 shortcut: Parakeet-v3-he int8 needs **no code**. It is the same
`nemo_transducer` engine and model-dir layout as our English Parakeet. Use it to put
Hebrew in front of the user this week, then move to the zipformer once the online
engine exists.

**None of the Hebrew models can replace Parakeet for English** (measured, §3):
pure-English WER is 36–59%, and many English clips come out in Hebrew script. Hebrew
has to be a per-language route, not a new default model.

## 1. Where we are

* ASR today (`DECISIONS.md` D4/D23):
  * English: Parakeet-unified-en-0.6b int8 on sherpa-onnx (`murmur-asr-sherpa`, offline `nemo_transducer`). Parakeet v2 is the fallback and the Android model.
  * Other languages or auto-detect: whisper.cpp base q5_1, with small q5_1 as an opt-in.
  * Streaming: D22 pause-segmented decoding, which works with any offline engine. A sherpa `OnlineRecognizer` engine is planned (D23 "next", for Nemotron) but does not exist yet: there is no `OnlineRecognizer` in `crates/`.
  * The Android settings already list `he` as a language.
* `docs/proposals/stt-model-survey.md` (2026-10-23) already flagged that **no sherpa package covered Hebrew** at the time. It named the ivrit.ai zipformer and the ivrit turbo ggml as untested options. This doc tests them.
* "Our CPU":
  * murmur-server box: Intel i7-6700 (Skylake 2015), 4 cores / 8 threads, AVX2/FMA, no AVX-512, 19 GB RAM, no GPU (`lscpu`/`free` on 2026-10-28; matches `docs/model-benchmarks.md`).
  * Windows PC: the repo docs do not record its hardware. **Assumption:** a desktop x86-64 CPU at least as fast as the i7-6700, and no usable GPU unless the user says otherwise.
  * Phone: Galaxy S25+ (survey doc).

## 2. The field (leaderboard, all rows that matter)

ivrit.ai leaderboard `benchmark.csv` [1], last commit 2026-10-14. Datasets:
* eval-d1: about 2 h of crowd-transcribed podcasts [10];
* eval-whatsapp: voice notes;
* saspeech;
* FLEURS he: read Wikipedia sentences, rare names and numbers;
* kan: broadcast.

| model | open? | d1 | whatsapp | saspeech | FLEURS | kan |
|---|---|---|---|---|---|---|
| Soniox (cloud) | no | 4.8 | 9.0 | 6.5 | 17.7 | 11.4 |
| **ivrit-ai whisper-large-v3 2025-05-13** | yes | **5.1** | 7.2 | **6.4** | **17.4** | **8.1** |
| **ivrit-ai whisper-large-v3-turbo 2025-05-13** | yes | 5.3 | **7.1** | 6.6 | 18.1 | 8.2 |
| **zipformer2-66m-streaming-he** (sherpa int8) | yes | 5.3 | 8.8 | 6.4 | 23.4 | 11.8 |
| parakeet-tdt-0.6b-v3-he (model card [8], not on the board) | yes | 6.1 | 8.8 | — | ≈ 23.2 ("76.8" accuracy) | — |
| Amazon Transcribe batch (cloud) | no | 6.6 | 10.4 | 8.5 | 23.0 | 9.0 |
| Deepgram nova-3 (cloud) | no | 6.7 | 12.0 | 10.2 | 23.3 | 21.0 |
| OpenAI gpt-4o-transcribe (cloud) | no | 7.3 | 12.6 | 10.9 | 21.0 | 39.4 |
| openai/whisper-large-v3-turbo (stock) | yes | 8.4 | 12.8 | 10.4 | 28.9 | 15.6 |
| openai/whisper-large-v3 (stock) | yes | 9.8 | 13.2 | 9.4 | 26.2 | 13.4 |
| Google Speech (cloud) | no | 21.1 | 35.2 | 18.9 | 38.5 | 29.2 |
| NVIDIA nemotron-3.5-asr-streaming-0.6b | yes | 17.1 | 36.5 | 24.2 | 46.0 | 54.2 |

Checked and ruled out (2026-10-28):
* **No Hebrew** in the model cards of Qwen3-ASR [12], Cohere Transcribe 03-2026, Voxtral Mini, or Canary-1b-v2 (25 European languages) [13]. Parakeet v3 is also European-only.
* **dm15/parakeet-tdt-0.6b-hebrew**:
  * v4 (Hebrew only): 7.9 / 11.4 / 24.7 on d1 / whatsapp / FLEURS [14].
  * The bilingual **v7** is built for English inside Hebrew, but its English WER is 23.8% on its own 20-clip test [15]. Worth watching; it is not good enough to replace two models.
* Meta Omnilingual ASR covers Hebrew, but it has no published Hebrew WER on these sets.

Takeaways:
* The ivrit.ai fine-tunes cut stock whisper's Hebrew WER by roughly 40% (d1 8.4 → 5.3 for turbo). The ivrit.ai Interspeech 2025 paper reports 8–29% lower error than other whisper versions [10].
* On conversational speech (d1 and whatsapp), which is closest to dictation, the 66M zipformer is as good as the 0.8B turbo. It falls behind only on read encyclopedic text (FLEURS, kan).

## 3. Measured on this box (2026-10-28)

Setup:
* i7-6700, 4 threads, wrapped in `flock … choom -n 1000 -- nice -n 10`. The box was shared, with a 1-minute load average of 3–10.
* Audio: 50 clips, every 6th unique sentence of FLEURS he_il **dev**. 413 s in total, 836 words, mean 8.3 s, range 1.3–17.8 s.
* Decoders:
  * sherpa models: the existing warm-recognizer probe `tools/probes/prototypes/stt_survey_2026_10_23` (`online` / `transducer` kinds, greedy);
  * whisper: the existing whisper.cpp `whisper-cli` with `-l he -bs 1 -bo 1`.
* WER normaliser: strip nikud and punctuation, then split on whitespace. It is close to, but not the same as, the leaderboard's.

| model | FLEURS-dev-50 WER | compute RTF | latency |
|---|---|---|---|
| zipformer2-66m-streaming-he int8 (online, 100 ms feed) | **26.8%** | **0.050** | **48 ms** mean (max 132 ms) after the last audio. The rest (≈ 370 ms per clip) runs while the user speaks. Load 3–5 s. |
| parakeet-tdt-0.6b-v3-he int8 (offline) | 28.8% | 0.069 | 572 ms mean (max 1151 ms) per clip after release. Load 2–8 s. |
| whisper small q5_1, `-l he` (today's "accurate" option) | 51.3% | 0.60 | ≈ 4.9 s per clip |
| whisper base q5_1, `-l he` (today's multilingual default) | **66.5%** | 0.19 | ≈ 1.5 s per clip |

* The two sherpa models land where their published FLEURS numbers say they should: 23.4 on the full test set, and 26.1 for the krut42 int8 on 100 dev clips [9]. So the ranking from §2 holds for the int8 builds on our CPU.
* **Today's whisper base/small fallback is unusable for Hebrew** (51–67% WER). A user who picks Hebrew today gets garbage.

**English through the Hebrew models.** Same probe, our 40-clip LibriSpeech set (Parakeet v2 scores 1.2% WER on it):

| model | English WER | clips that came out (partly) in Hebrew script |
|---|---|---|
| zipformer-he | 58.7% | 23 / 40 |
| parakeet-v3-he | 35.5% | 14 / 40 |

The models often transliterate or even translate English into Hebrew. An example from parakeet-he: "Alice took up the fan…" → "אליס עקוב את הפן…".

**Code-switching** (4 synthetic Hebrew + English concatenations, 0.3 s gap):
* Both models switched script correctly at the sentence boundary in 3 of 4 clips.
* In the 4th clip the zipformer *translated* the English sentence into Hebrew.
* Parakeet-he kept English better but dropped a few words.

Conclusion: sentence-level mixing is partly OK. A pure-English dictation must not go through a Hebrew model.

**Not measured, estimated:** ivrit whisper turbo and large-v3 on CPU. The ggml files are 1.6 GB and 3.1 GB, over this ticket's 1.5 GB download cap.
* Turbo has exactly the stock large-v3-turbo architecture. Stock turbo q5_0 was measured here on 2026-10-20 at RTF 3.26, about 25 s for any clip, because the encoder always processes a 30 s window. With `-ac` it is about RTF 1, but that needs a repetition guard (`docs/model-benchmarks.md`).
* The published ivrit ggml is f16, so it would be slower until it is quantized to q5_0 (≈ 550 MB).
* large-v3 has the same encoder plus a decoder 8× deeper (32 vs 4 layers), so it costs more than turbo: ≥ 40 s per dictation here (**estimate**).
* A third-party project reports ivrit turbo "at about 1x" real time on CPU [16], which is consistent with the above.

## 4. Recommendations

### 4.1 Best for our CPU: zipformer2-66m-streaming-he (int8, sherpa-onnx)

* **Accuracy:**
  * 5.3 / 8.8 on d1 / whatsapp, against 5.3 / 7.1 for ivrit turbo, the best model that would be interactive on a GPU.
  * It is weaker on read encyclopedic text (FLEURS 23 vs 18): rare names, numbers, Latin terms.
* **Speed (measured):**
  * RTF 0.05, about 2.5× cheaper than our English Parakeet (RTF 0.12–0.14).
  * The text is ready about 50 ms after the user stops, at any utterance length. This is the "streaming" property D23 wanted from Nemotron, with a Hebrew model.
* **Size:** 72 MB int8, so it also fits the Windows app and the phone. The model card says the int8 build runs in a browser via WASM [7].
* **Cost:** a new `OnlineRecognizer` engine in `murmur-asr-sherpa`. This is the same work D23 already queued for Nemotron, so one adapter serves both.
  * The probe's `online` kind is the reference: feed chunks, then flush with about 1.5 s of silence and `input_finished()` [7].
  * `model_type` is auto-detected from the ONNX metadata (zipformer2).
  * Until the adapter exists, the engine can be fed the whole buffer at commit, which is still RTF 0.05.

### 4.2 Best ignoring compute: ivrit-ai/whisper-large-v3

* It has the lowest open-model WER on d1, saspeech, FLEURS and kan, and is within 0.1 of turbo on whatsapp [1].
* **Hardware:** an NVIDIA GPU. Reference figure for the same-size large-v2 under faster-whisper: 4.5 GB VRAM at fp16, 2.9 GB at int8 [11].
* **Turbo** is within 0.7 points (0.1 better on whatsapp) and is about 2× faster. Measured p50 latency [2]:

  | GPU | 5–15 s clip (p50) |
  |---|---|
  | RTX 5090 | 0.38 s |
  | RTX 2080 Ti | 0.50 s |
  | T4 | 0.68 s |

  For a real deployment on a GPU box, **turbo** is the sensible choice. Run it through whisper.cpp built with CUDA, which uses our existing `whisper-rs` path, or through faster-whisper/CT2.
* **Caveat from the model card:** language detection was degraded by fine-tuning. Always pass `language = he` [5].

### 4.3 Integration notes and risks

* **Routing, not replacement.** `AsrRequest.language` already exists in `murmur-core`.
  * `he` → Hebrew engine; `en` → Parakeet (unchanged).
  * Auto mode needs language ID before decoding. Options: whisper tiny/base LID on the first ~2–3 s (whisper.cpp is already in core), or sherpa-onnx's whisper-based spoken-language-identification binary, which is already in the cached sherpa build.
  * Per-segment routing with D22 pieces would handle sentence-level mixing.
* **Step 0 (no code):** ship Parakeet-v3-he int8 as a model directory for the existing `nemo_transducer` engine, with `language = he` routing. It costs 0.57 s per dictation on the server, and is still 2× better than whisper small.
* **Step 1:** add the sherpa `OnlineRecognizer` engine, then make the zipformer the Hebrew default on server, Windows and Android.
* **Step 2 (GPU tier, if the user ever has one):** ivrit turbo on whisper.cpp with CUDA.
* **Downstream is English-only today:**
  * the deterministic pass (fillers, spoken punctuation, backtrack phrases, number ITN);
  * the few-shot cleanup prompt;
  * the D25 sentence-case rule.

  All of these need Hebrew versions or must be bypassed for `he`. Paste needs RTL-safe text (no stray bidi marks).
* **Numbers:** the Hebrew models write numbers inconsistently, as words or digits [8].
* **Hotwords:** the personal dictionary uses sherpa hotwords. Whether that works with the Hebrew BPE vocabularies is untested.
* **Licences:**
  * the models are Apache-2.0 / CC-BY-4.0 (attribution required);
  * the training audio is under ivrit.ai's own dataset licence [7], which matters only if we fine-tune.
* **Eval caveats:**
  * our FLEURS-50 sample is small (836 words) and is read speech;
  * the code-switch test is 4 synthetic clips;
  * no real Hebrew dictation from the user has been scored;
  * the next useful check is 10–20 of the user's own Hebrew dictations through Parakeet-he and the zipformer.

## Reproduce

* Downloads went to a temp dir that was deleted afterwards:
  * models from [7] and [9];
  * FLEURS `he_il/audio/dev.tar.gz` + `dev.tsv` [17].
* Audio was converted from float32 to 16 kHz PCM16 with the cached venv's `soundfile`.
* Decoding used the existing probe binary:
  * `stt_survey_2026_10_23 online <zip-dir> <manifest> <audio> --threads 4`
  * `stt_survey_2026_10_23 transducer <parakeet-he-dir> …`
  * `whisper-cli -l he -t 4 -bs 1 -bo 1 -nt -otxt` for the whisper rows.
* No new code was committed (research-only ticket).

## Sources (accessed 2026-10-28)

1. ivrit.ai Hebrew transcription leaderboard, `benchmark.csv`: https://huggingface.co/spaces/ivrit-ai/hebrew-transcription-leaderboard/blob/main/benchmark.csv (space: https://huggingface.co/spaces/ivrit-ai/hebrew-transcription-leaderboard; last commit 2026-10-14)
2. Same space, GPU latency table `timing.csv`: https://huggingface.co/spaces/ivrit-ai/hebrew-transcription-leaderboard/blob/main/timing.csv
3. https://huggingface.co/ivrit-ai/whisper-large-v3
4. https://huggingface.co/ivrit-ai/whisper-large-v3-ggml (3.1 GB f16)
5. https://huggingface.co/ivrit-ai/whisper-large-v3-turbo (training date Apr 2025; "language token should be explicitly set to Hebrew")
6. https://huggingface.co/ivrit-ai/whisper-large-v3-turbo-ggml (1.62 GB f16; uploaded 2025-05-22, the card does not name the checkpoint)
7. https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he (architecture, 1.28 s chunk, WER table, int8 5.3 / 8.8, sherpa usage, CC-BY-4.0)
8. https://huggingface.co/thewh1teagle/parakeet-tdt-0.6b-v3-he (6.1 / 8.8, FLEURS "76.8", limitations)
9. https://huggingface.co/krut42/voice-parakeet-he-int8 (sherpa-onnx int8 export, FLEURS dev 100 clips 26.1%)
10. Marmor et al., "Building an Accurate Open-Source Hebrew ASR System through Crowdsourcing", Interspeech 2025: https://www.isca-archive.org/interspeech_2025/marmor25_interspeech.pdf
11. faster-whisper README, large-v2 GPU VRAM table: https://github.com/SYSTRAN/faster-whisper#large-v2-model-on-gpu
12. https://huggingface.co/Qwen/Qwen3-ASR-0.6B (language list, no Hebrew)
13. https://huggingface.co/nvidia/canary-1b-v2 (no Hebrew)
14. https://github.com/danielm1515/parakeet-tdt-0.6b-hebrew (v4 results)
15. https://huggingface.co/dm15/parakeet-tdt-0.6b-hebrew/blob/main/HEBREW_ENGLISH_V7.md
16. https://github.com/am-consultingai/upshot/pull/5 ("Hebrew on the CPU at about 1x … ivrit turbo"; third-party)
17. https://huggingface.co/datasets/google/fleurs (he_il)
