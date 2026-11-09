# 2026-11-02: Pixel offline dictation, and how Flow Clone could match it in English

**Question (owner).** Hebrew is fine; English is not. Pixels have dictation with AI cleanup that is
great even offline. What is it, can we use it on the S25 Edge (SM-S936B, One UI 9 / Android 17), and
which open models could match it?

## TL;DR (ranked)

1. **Spike the ML Kit GenAI Speech Recognition API, advanced mode, on the S25 Edge.** It is Google's
   own Gemini Nano speech model, the Pixel 10 engine, and its device list names our chipset
   (**SM8750**). Gain: Pixel-class English with no model of ours. Effort: about 1 day for a spike,
   about 3 days for an engine behind the router.
   * **Risk:** GenAI inference is allowed **only when the calling app is the top foreground app**.
     Our accessibility overlay dictates into *other* apps, so it may be refused with
     `BACKGROUND_USE_BLOCKED`. The spike has to answer this first.
2. **Cheap wins with the stack we have (no new model).**
   * Make biasing real: dictionary, screen words and the app name go into sherpa hotwords on every
     local English decode.
   * Run AI cleanup by default, with a "fix misheard technical terms" instruction.
   * Build a small English set of real-style dictation (synthetic text, owner's voice) so progress
     can be measured.
   * Gain: fixes the "pie test" → "pytest" class of errors, which a WER swap won't. Effort: 1–3 days.
3. **Fallback if (1) is blocked:** the platform `SpeechRecognizer` on-device recognizer.
   * Feed it our own audio through `EXTRA_AUDIO_SOURCE`, with `EXTRA_BIASING_STRINGS` and
     `EXTRA_ENABLE_FORMATTING`.
   * It has no foreground-only rule, but it is probably Google's older on-device model.
   * Effort: about 2 days.

**Not recommended: swapping the open model.** The best open English models are only about 5 %
relative better than our Parakeet-unified on the Open ASR set (5.63 against 5.91 average WER), and
they need a GPU. Our own LibriSpeech numbers are already 1.0–1.2 % (2026-10-30, 2026-10-31 benches).
The English gap is context, vocabulary and cleanup, not acoustic WER.

## 1. What the Pixel feature is

| generation | feature | ASR | cleanup |
|---|---|---|---|
| Pixel 3+ (2019) | Gboard "Faster voice typing", offline | all-neural on-device RNN-T, about 80 MB, en-US first ([Google Research](https://research.google/blog/an-all-neural-on-device-speech-recognizer/)) | none (the model punctuates) |
| Pixel 6+ (Tensor) | "Assistant voice typing": voice edits ("delete", "send"), auto-punctuation | larger on-device model on the TPU (press only; no primary source names it) | rules and commands |
| Pixel 10 | same, plus the **ML Kit GenAI speech "advanced" mode** for apps | "the GenAI model … better overall quality", reached through AICore / Gemini Nano ([ML Kit speech](https://developers.google.com/ml-kit/genai/speech-recognition/android)) | — |
| Pixel 11 (Aug 2026) | **Gboard Rambler**: removes fillers, handles mid-sentence corrections and code-switching, edits by voice ([Gboard help](https://support.google.com/gboard/answer/17468539)) | not named ("powered by Gemini") | Gemini |

* **Offline vs online (Rambler).** Offline, "Basic cleanup, punctuation, and capitalization remain
  active". Advanced rewrites and complex voice edits need a connection, and a *Retry* sends the
  input again when the connection is back.
* So **offline means an on-device ASR (Gemini Nano class on Pixel 10/11) plus light on-device
  cleanup**, and the heavy rewrite is cloud Gemini. That is our own split (local ASR + rules,
  cloud LLM).
* Google has not published the model's architecture or size. Press reports that Rambler uses
  "Gemini Nano v3" are unconfirmed.

## 2. Can Flow Clone use it on the S25 Edge?

### 2a. ML Kit GenAI Speech Recognition (`com.google.mlkit:genai-speech-recognition:1.0.0-alpha1`) ([docs](https://developers.google.com/ml-kit/genai/speech-recognition/android))

* **Modes.**
  * **Basic:** "the traditional on-device model, similar to the platform SpeechRecognizer". It
    runs on API 31+. It has 15 locales, and only en-US is not beta.
  * **Advanced:** the GenAI model, on "Pixel 10 and Pixel 11 series, plus devices with Gemini Nano
    on" **SM8750**, SM8845, SM8850, SM8950, SM8975, Dimensity 9400/9500-class and Exynos 2600.
    It has 21 locales, en-US among them. Hebrew is not listed, which is fine because Hebrew stays
    on our models.
* **Interface.**
  * Streaming partials ("may initially be partial (and subject to change)").
  * Input from the mic, a file, or **our own PCM via `fromPfd()`** (16 kHz, 16-bit mono). That PCM
    path fits our capture pipeline.
* **Limits** ([GenAI overview](https://developers.google.com/ml-kit/genai)):
  * "inference is permitted only when the app is the top foreground application … including using
    a foreground service, will result in `BACKGROUND_USE_BLOCKED`";
  * a per-app inference quota (`BUSY`) and a per-app battery quota;
  * no support on devices with an unlocked bootloader;
  * alpha, "not subject to any SLA or deprecation policy";
  * the ML Kit GenAI Additional Terms apply.
* **S25 Edge status: unknown.** The overview lists the S25 / S25+ / S25 Ultra for the text and
  image features but not the Edge. The speech page goes by chipset ("devices with Gemini Nano"),
  so whether it works depends on AICore holding a Nano model on this phone. This needs an on-phone
  check.
* **Prompt API (Gemini Nano text).** It is listed for the S26 (nano-v3), not the S25. So an
  on-device Nano cleanup on the owner's phone is unlikely, and the Qwen fallback stays.

### 2b. Platform `SpeechRecognizer.createOnDeviceSpeechRecognizer` ([reference](https://developer.android.com/reference/android/speech/SpeechRecognizer), [RecognizerIntent](https://developer.android.com/reference/android/speech/RecognizerIntent))

* **API levels.**
  * API 31 adds `createOnDeviceSpeechRecognizer`. It throws if `isOnDeviceRecognitionAvailable`
    is false.
  * API 33 adds `checkRecognitionSupport`, `triggerModelDownload`, `EXTRA_BIASING_STRINGS`,
    `EXTRA_ENABLE_FORMATTING`, `EXTRA_SEGMENTED_SESSION` and `EXTRA_AUDIO_SOURCE` (the app
    supplies the audio).
  * API 34 adds language detection and language switching.
* **Rules.** It must be called on the main thread and needs `RECORD_AUDIO`. Its docs warn it is
  "not intended to be used for continuous recognition"; segmented sessions are the supported way
  to do long dictation. Each extra "may have no effect" depending on the implementation.
* **Which engine backs it on Samsung** (Google's Speech Recognition & Synthesis / Android System
  Intelligence, or Samsung's own) is set by the system and not documented, so it needs an on-phone
  check.
* **Who can call it.** An accessibility service or an IME can call it (it is an ordinary app API).
  With `EXTRA_AUDIO_SOURCE` we keep our own mic capture, VAD and endpointing, and the
  recognizer's start/stop beeps should not apply (unverified).
* **Limits.** No foreground-only rule is documented.

### 2c. Gboard / Rambler itself

* No app API exists. The only way to use it is to switch to Gboard as the keyboard, which replaces
  Flow Clone.
* As of late 2026, Rambler is listed for the Pixel 11 only. Press says wider Android rollout will
  follow (unconfirmed).

## 3. Open-weight alternatives

**How the numbers were made.** The 8-set averages are computed from each model card's
Open-ASR-style figures: AMI, Earnings-22, GigaSpeech, LibriSpeech clean and other, SPGI, TED-LIUM
and VoxPopuli. The live leaderboard data is gated
([space](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard),
[paper](https://arxiv.org/abs/2510.06961)). All cards were read on 2026-11-02.

| model (card date) | size | avg WER | streaming | Android runtime | notes |
|---|---|---|---|---|---|
| [parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) (2026-04), **ours** | 0.6B | **5.91** offline, 6.29 at 1.12 s | **yes** (one model) | sherpa ✓ (240 / 560 / 1120 ms streaming packages exist) | NVIDIA Open Model License |
| [parakeet-tdt-0.6b-v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) | 0.6B | 6.05 | no | sherpa ✓ | CC-BY-4.0 |
| [parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) | 0.6B | 6.34 | no | sherpa ✓ | 25 languages |
| [canary-qwen-2.5b](https://huggingface.co/nvidia/canary-qwen-2.5b) | 2.5B | **5.63** | no | none (NeMo, GPU) | best fully published average |
| [granite-speech-4.1-2b](https://huggingface.co/ibm-granite/granite-speech-4.1-2b) (2026-04) | 2B | ~5.3 (plot only; secondary sources) | no | none (vLLM) | **keyword-list biasing**; Apache-2.0 |
| [Qwen3-ASR-1.7B / 0.6B](https://huggingface.co/Qwen/Qwen3-ASR-1.7B) (2026-01) | 1.7B / 0.6B | not on the same 8 sets (LS 1.63 / 3.38, Giga 8.45) | vLLM only | sherpa ✓ (0.6B int8) | 52 languages |
| Cohere Transcribe (2026-03) | 2B | 0.84 % against 1.15 % for v2 on **our** set; 2.5× slower and dropped an utterance at 56 s (`docs/proposals/stt-model-survey.md`) | no | sherpa ✓ int8 | |
| [nemotron-speech-streaming-en-0.6b](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b) | 0.6B | 6.94 at 1.12 s | yes | sherpa ✓ | weaker than unified-streaming |
| [moonshine-streaming-medium](https://huggingface.co/moonshine-ai/moonshine-streaming-medium) (2026-01) | 245M | 6.65 (7 sets, no TED) | yes | own runtime, [Android](https://github.com/moonshine-ai/moonshine) | MIT; small; hallucination risk (seq2seq) |
| [Voxtral-Mini-4B-Realtime-2602](https://huggingface.co/mistralai/Voxtral-Mini-4B-Realtime-2602) | 4B | not published on these sets | yes, 80 ms – 2.4 s delay (480 ms recommended) | none (vLLM, ≥ 16 GB GPU) | Apache-2.0; server with a GPU only |
| [kyutai stt-2.6b-en / 1b-en_fr](https://huggingface.co/kyutai/stt-2.6b-en) | 2.6B / 1B | — | yes, 2.5 s / 0.5 s delay | none (GPU, Rust/MLX) | semantic VAD |
| whisper large-v3 / turbo | 1.5B / 0.8B | 7.44 (v3, as cited by Moonshine) | no | sherpa / whisper.cpp | our laptop: turbo 1.7 % against Parakeet 1.2 %, 14.5 s per clip on the CPU |

**Takeaways.**
* Nothing that runs on the phone beats what we ship.
* On the server, Canary-Qwen and Granite gain about 0.3–0.6 points of average WER and need a GPU.
* The free win on the phone is **latency**: the *same* unified model ships as a streaming
  package (6.29 at 1.12 s), which would remove the offline commit decode (p50 973 ms on the phone).

## 4. Why Wispr and Pixel feel better than WER says

1. **Context biasing.**
   * Wispr sends dictionary terms, screen words, OCR words, the user's names and file names to bias
     its ASR.
   * Google's ASR has biased toward contacts and app context for years
     ([Pundak et al., "Deep context"](https://arxiv.org/abs/1808.02480)).
   * Our case: "pie test" for "pytest" and "a UID four" for "a UUID4"
     (`2026-11-02-local-cleanup-skipped.md`).
   * **Cheap:** we already have `dictionary_terms` and `screen_terms` and sherpa hotwords
     ([docs](https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html)). Verify that both reach the
     local Parakeet decode on the phone, and tune the hotword score.
2. **LLM cleanup that fixes terms.**
   * Wispr's formatter sees the same context and corrects misheard words.
   * Ours was off on the phone by default (D43 fixes the order), and the 1.5B is too slow (11 of
     about 50 time out at 3 s).
   * **Cheap:** run cloud/server cleanup always when online, and give the prompt the screen and
     dictionary terms with permission to correct near-homophones.
3. **Endpointing and VAD.** Quiet speech was dropped by absolute gates (fixed 2026-11-01), and
   first words were cut on the streaming path (2026-10-31). Both read as "inaccurate" to a user.
   Already fixed or tracked.
4. **Punctuation and casing.** Parakeet already punctuates. The gap is style, which the LLM
   handles.
5. **Personalization.** "Learn from my edits" exists. Feeding the learned words into the hotwords
   is cheap.
6. **Test set.** LibriSpeech read speech hides all of the above. A 30-clip set of real-style
   English dictation (tech terms, names, app text) is the cheapest way to tell whether (1) or (2)
   moves anything.

## Open questions → phone-queue items

* **PQ-a, the GenAI speech gate (highest).**
  * Needs: a debug build with a probe screen that calls the ML Kit speech `checkFeatureStatus`
    in advanced mode, en-US.
  * Steps: record `AVAILABLE`, `DOWNLOADABLE` or `UNAVAILABLE`; download if downloadable; dictate
    one synthetic sentence **from the overlay into another app** and **from the app itself**.
    Log `BACKGROUND_USE_BLOCKED` or the text, the latency, and `getBaseModelName`.
* **PQ-b, AICore and bootloader.**
  * Check whether `com.google.android.aicore` is installed, and at which version.
  * Check that the bootloader is locked (`ro.boot.flash.locked` / `ro.boot.verifiedbootstate`).
* **PQ-c, the on-device recognizer.**
  * Read `settings get secure voice_recognition_service` and the on-device recognition service
    component (dumpsys of `speech_recognition`).
  * Record `isOnDeviceRecognitionAvailable` and `checkRecognitionSupport(en-US)`: installed or
    pending languages.
* **PQ-d, platform recognizer quality.**
  * Same probe screen: `createOnDeviceSpeechRecognizer` with `EXTRA_AUDIO_SOURCE` (our PCM),
    `EXTRA_BIASING_STRINGS` (3 synthetic terms) and `EXTRA_ENABLE_FORMATTING`.
  * Note whether partials arrive, whether a beep plays, and the session cutoff on a 60 s clip.
* **PQ-e, real-style English set.** The owner records 30 synthetic scripted lines (tech terms,
  names). The audio goes under `phone-queue/pulled/` and is scored on the three engines (Parakeet,
  GenAI advanced, platform). The audio never enters the repo.
