# Hebrew speech-to-text (0.10.0)

Research: `docs/findings/2026-10-28-hebrew-stt-models.md`. Today a user who picks Hebrew
gets whisper base/small at 51–67 % WER; Parakeet ignores the language and outputs
English.

## Decisions

1. **Per-language model routing, not a new default.** The Hebrew models score 36–59 %
   WER on English (and often write English in Hebrew script), so English keeps Parakeet.
   `ModelConfig` gains an optional per-language model map; a dictation whose forced
   language (`SessionConfig::forced_language()`) has an entry runs on that model,
   everything else on the default engine, unchanged.
2. **Default Hebrew model: `ivrit-ai/zipformer2-66m-streaming-he` int8** (72 MB,
   CC-BY-4.0). Measured here: 26.8 % FLEURS-dev WER, RTF 0.05, beating Parakeet-v3-he
   int8 (28.8 %, 673 MB) on both accuracy and size. Parakeet-v3-he
   (`krut42/voice-parakeet-he-int8`) stays selectable: it loads on today's
   `nemo_transducer` path with no engine code.
   *Confirmed by H7* on the owner's own dictations (13.6–14.4 % vs 18.9 % WER,
   ≈ 50 ms vs ≈ 420 ms after the last audio):
   `docs/findings/2026-11-02-h7-hebrew-stt-scores.md`.
3. **The zipformer needs a sherpa *online* engine.** sherpa-onnx 1.13.8 (already a
   dependency) has `OnlineRecognizer` / `OnlineStream`. First version feeds the whole
   buffer at commit, then ~1.5 s of silence and `input_finished()` (still RTF 0.05).
   Live chunk feeding is a later, app-wide improvement, not part of 0.10.0.
4. **One model in memory where RAM is tight.** Android loads only the model for the
   current language (switching language reloads). Windows and the server may keep the
   default and the Hebrew model loaded.
5. **English-only post-processing is skipped for non-English output.** The rules pass
   (`postprocess::spoken::spoken_cleanup`: fillers, spoken punctuation, backtrack,
   commands, ITN) and `layout()` and `segment::capitalize_joins` run only when the
   dictation language is English (or unknown on an English engine). Kept for all
   languages: whitespace tidy, `terminal::close_sentence` (script-neutral), dictionary
   replace, press-enter. Guard: English word lists don't apply to Hebrew; numeric rules
   stay. AI cleanup prompts already say "keep the speaker's language" and name the
   language when known.
6. **Auto-detect stays English-first.** With language = auto, dictation uses the default
   engine as today. Hebrew needs the language set to Hebrew until the language-ID task
   (whisper LID on the first seconds) lands. *Landed as H6 / D34:* with `lid_path` set,
   auto-detect identifies the first 3 s and routes Hebrew to the Hebrew model.
7. **Engines report the language they decoded**: Parakeet-he / zipformer-he return
   `he`, the English Parakeet keeps `en`.
8. Paste needs no change: Windows sends one UTF-16 string (or logical-order key events),
   Android one `commitText`; no bidi marks are added.

## Tasks (TickTick "Murmur", tag `0.10.0`)

| # | Task | Blocked by |
|---|---|---|
| H1 | Core: per-language model map + routing, engines report language, language-gated post-processing | — |
| H2 | murmur-asr-sherpa: online (zipformer2) engine + core backend kind | — |
| H3 | murmur-server: Hebrew model config, `fetch-models.sh --hebrew` | H1, H2 |
| H4 | Windows: language picker with Hebrew, Hebrew model in Settings → Speech + download catalogue (zipformer-he, parakeet-he) | H1, H2 |
| H5 | Android: Hebrew in Speech settings, Hebrew model import, load-by-language | H1, H2 |
| H6 | Auto-detect Hebrew (language ID before decode) | H1 |
| H7 | Score 10–20 of the owner's own Hebrew dictations on both models. *Done:* `docs/findings/2026-11-02-h7-hebrew-stt-scores.md` (16 clips: zipformer-he 13.6–14.4 % WER vs parakeet-he 18.9 %; decision 2 holds) | owner recordings |
