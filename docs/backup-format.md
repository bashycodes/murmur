# Backup file format (`murmur-backup`, version 1)

Settings → **Back up & restore** on Android writes every piece of user data
into one UTF-8 JSON file, so data survives an install that wipes it
(uninstall, a signing-key change, a factory reset). The Windows app is meant
to read and write the same format later; sections it does not have are left
out, and the reader leaves those stores alone.

Code: `apps/murmur-android/app/src/main/java/dev/murmur/data/Backup.kt`
(format, pure) and `BackupIo.kt` (Android stores). Tests: `BackupTest`,
`BackupIoTest`.

## File

Default name `murmur-backup-YYYY-MM-DD.json`; the user picks where it
goes (Storage Access Framework, no storage permission).

```json
{
  "format": "murmur-backup",
  "version": 1,
  "app_version": "0.7.1",
  "platform": "android",
  "flavor": "hybrid",
  "created": "2026-10-29T10:00:00+02:00",
  "data": {
    "settings":          { "<key>": { "type": "string", "value": "he" } },
    "developer":         { "<key>": { "type": "long",   "value": 8765 } },
    "dictionary":        [ { "word": "brb", "replacement": "be right back", "isSnippet": true } ],
    "learn_suggestions": { "pending": [ ... ], "dismissed": [ ... ] },
    "insights":          { "backfilled": true, "days": { "2026-10-28": { "words": 12, "ms": 3000, "count": 2, "apps": { "com.chat": 12 } } } },
    "notes":             [ { "id": "scratchpad", "text": "...", "pinned": true, "created": 1730000000000, "updated": 1730000000000 } ],
    "history":           [ { "created_at": 1730000000000, "app_id": "com.chat", "status": "FORMATTED", "asr_text": "...", "final_text": "...",
                             "inserted": true, "audio_ms": 1200, "fallback_reason": null, "asr_ms": 300, "llm_ms": 0, "total_ms": 900,
                             "insert_outcome": "inserted", "route": "local", "command_original": null } ]
  }
}
```

| Section | Android store | Shape |
|---|---|---|
| `settings` | SharedPreferences `murmur_settings` | key → `{type, value}`; type is `string`, `bool`, `int`, `long`, `float` or `string_set` (value = array of strings) |
| `developer` | SharedPreferences `dev` (routing, server address, OpenRouter config, usage) | as `settings` |
| `dictionary` | `files/dictionary.json` | the shared dictionary export `[{word, replacement?, isSnippet}]` |
| `learn_suggestions` | `files/learn_suggestions.json` | as stored |
| `insights` | `files/insights.json` | the Insights file shape shared with Windows (`crates/murmur-core/src/insights.rs`) |
| `notes` | `files/notes/` (`<id>.txt` + `index.json`; includes the Scratchpad / Quick note, id `scratchpad`) | id (`[a-z0-9][a-z0-9_-]{0,63}`), text, pinned, created/updated epoch ms |
| `history` | `history.db` table `history` (v4 columns), oldest first | one object per row; row ids are not kept |

Preference keys are the stores' own keys, so the meaning of a key is defined
by the app version that wrote it; app migrations (e.g. the endpoint default)
run on restored values as on any stored ones.

## Never in a backup

- Secrets: the server token (`server.token_sealed`), cloud API keys
  (`cloud.key_sealed.<id>`) and any key whose name contains token, secret,
  password, key_sealed, api_key or credential. They are Keystore-sealed and
  could not be opened on another install anyway. The filter runs on export
  and again on import (a hand-edited file cannot plant one).
- Device-local `murmur_settings` values: `model_path`, `parakeet_path`,
  `llm_model_path`, `service_stopped_by_user`.
- Models (`files/models`, `files/parakeet`, `files/llm`), the debug log and
  debug audio (`files/debug/`).

After a restore the user enters API keys and the server token again and
re-imports models.

## Reading rules

1. Refuse unless `format` is `murmur-backup` and `version` is an integer
   ≥ 1. A `version` above the reader's own is refused with "made by a newer
   version, update the app" (never half-read).
2. Parse and validate **every** section before writing anything: a malformed
   section, a note with an invalid or duplicate id, or a setting with an
   unknown type rejects the whole file.
3. Restore is **replace**, per section: a section present in the file (even
   empty) replaces that store; a section absent (or `null`) leaves it alone.
   Secrets and device-local values already on the install are kept.
4. Each store is written atomically on its own (temp file + rename, the notes
   directory swapped by rename, History in one SQLite transaction). Stores are
   not one transaction together: an I/O failure half-way can leave some
   restored, which a second restore of the same file completes.

A new section or field is a compatible change (old readers ignore it). A
change old readers would misread bumps `version`.
