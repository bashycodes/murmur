# English dictation comes out as Hebrew (Android hybrid, 1.0.0)

Date: 2026-11-01. Branch: `feat/stt-lang-hebrew`.

## Symptom

The keyboard is set to English. The user dictates an English sentence (synthetic
example: "Where did you put the keys?"), and the app inserts a Hebrew translation
of it ("איפה שמת את המפתחות?") instead of the English text.

## Evidence (app logcat right after the bug, 6 dictations)

All transcript text below is synthetic. The field values are the real ones.

```
I/MurmurService: commit FORMATTED audio=5720ms asr=467ms llm=1002ms total=1472ms ... route=local/-
I/MurmurDebug: {"kind":"dictation","app_version":"1.0.0","build":"3c8df49","route":"local",
  "engine":"sherpa_online","model":"zipformer-he","language":"he","language_source":"fixed",
  "llm":"formatted","llm_engine":"cloud","llm_model":"meta-llama/llama-3.3-70b-instruct",
  "cleanup_route":"cloud","target":"com.whatsapp",
  "raw":"איפה שמת את המפתחות?","text":"איפה שמת את המפתחות? ", ...}
```

- All 6 dictations: `route=local`, `engine=sherpa_online`, `model=zipformer-he`,
  `language=he`, `language_source=fixed`. The murmur-server and cloud STT were not
  used, so the server's `languages = he` played no part.
- `raw` (the STT output, before cleanup) is already a fluent Hebrew sentence in
  5 of 6. Cleanup (cloud Llama 3.3 70B) returned it unchanged apart from the
  trailing space, and one dictation skipped cleanup (`llm_skipped_clean`). So
  cleanup did not translate anything.
- 1 of 6 came out in English from the same Hebrew model (it can emit Latin
  text), which shows the user was speaking English throughout.

## Root cause

1. **The language setting is a fixed Hebrew (`"he"`)**, so `LanguageDefault.resolve`
   returns `Resolved("he", FIXED)` and never reads the keyboard. Only the
   "Keyboard language" setting reads the keyboard subtype.
2. Before D37 (2026-10-31, shipped in 1.0.0) the only way to reach the Hebrew
   model (H5) was to set the language to Hebrew. The one-time D37 migration moved
   "Phone language" to "Keyboard language", but kept explicit languages on
   purpose (`DataLogicTest`: `"explicit Hebrew" → "he"`). The user's
   pre-1.0.0 Hebrew choice therefore survived the upgrade.
3. With language `he` and a Hebrew model installed, `HebrewModels.route` loads
   **only** the Hebrew model (`load_language = he`), so every dictation,
   including English ones, is decoded by the ivrit.ai Zipformer.
4. That model writes English speech as Hebrew text that reads like a translation.
   Its training labels presumably came from Whisper run with the language forced
   to Hebrew, which translates English speech. That is how it behaves, not
   something the app can fix in decoding.

## Proposed fix (reverted)

`feat/stt-lang-hebrew` added a one-time migration `Settings.migrateHebrewToKeyboard`
(flag `language_he_keyboard`) that turned a stored `"he"` into `"keyboard"`, plus a
D37 amendment. It was removed before release (`feat/drop-he-migration`); see Decision.

## Decision

- No migration. A fixed Hebrew is a deliberate user choice, and the app respects it.
  D37's original rule "explicit languages are kept" stands.
- The fix for a user with an English keyboard is Settings ▸ Language ▸ Keyboard
  language: an English keyboard then gets English, a Hebrew keyboard the Hebrew model.
- No warning when the keyboard language differs from a fixed language (the owner
  declined it).

## Not changed / follow-ups

- If Hebrew is picked as a fixed language, English speech still comes out in
  Hebrew, because that is how the model behaves. A mismatch warning was declined
  (see Decision).
- The keyboard language is read once at dictation start. A keyboard switch in the
  middle of a dictation is not seen.
