# Streaming-ASR (D22): what the shells must call and set

Status: frozen 2026-10-23 on branch `feat/streaming-asr` (core side:
`docs/proposals/streaming-asr-latency.md`). Everything below is additive JSON.
There are **no new C or JNI symbols**: `SessionConfig` already crosses
`flow_start_session` / `nativeStartSession` as JSON, and `CommitResult` comes
back as JSON. An old core or an old server ignores the new fields and still
returns the full text, without the speed-up.

## 1. Session config: three new optional fields

Send them on every `start_session` (router or bare core), in both PTT and
hands-free mode:

| JSON key | type | shell value | meaning |
|---|---|---|---|
| `segment_pause_ms` | u32 or absent | **800** when the pref is on; omit it when off | Decode each finished phrase while the user keeps talking. Cuts go in the middle of pauses ≥ this long. Absent = off, which is today's behaviour, byte-identical |
| `segment_min_ms` | u32 or absent | omit (core default 4000) | Pieces shorter than this keep growing |
| `segment_soft_max_ms` | u32 or absent | omit (core default 20000) | A piece this long is cut at its longest gap ≥ 200 ms in the last 5 s |

Constants: `flow_core::segment::RECOMMENDED_PAUSE_MS` = 800,
`DEFAULT_MIN_MS` = 4000, `DEFAULT_SOFT_MAX_MS` = 20000. Values below 200 are
raised to 200.

* Pref: a Developer/advanced toggle **"Decode while speaking"**, default
  **on**. Store it per device, not in the synced dictionary.
* The core turns it off by itself for engines that must not see audio
  before commit (cloud STT, D18). Shells don't special-case the cloud route.
* Live preview: no change. `partial()` / `poll().partial` already returns
  finished segments plus the open one's preview. A preview whose window
  starts after the open segment's start begins with `"… "` (window capped at
  12 s). Display it as is.
* Cancel (✕): no change. `cancel()` discards every decoded segment.

## 2. Hands-free endpoint: default 2000 → 1500 ms

`flow_core::endpoint::RECOMMENDED_SILENCE_MS` is now **1500**. Change the
shell default for the hands-free `endpoint_silence_ms` pref from 2000 to
1500. Keep the 300–10000 range. If a user already saved 2000 explicitly, keep
their value; only the default changes. PTT still leaves it unset.

* Android: `DataLogic.kt` default for the hands-free endpoint.
* Windows: `prefs.rs` (`hands_free_endpoint` default).

## 3. Results: new optional fields (read them for history/Developer rows)

`CommitResult.timings` gains these fields. Each is omitted from the JSON when
it is 0, so parse it as optional with default 0:

| field | meaning |
|---|---|
| `wait_ms` | `commit` waited this long for the worker to finish a decode already in flight (a preview or a segment). Included in `total_ms` |
| `segments` | Pieces that went to ASR (closed segments + tail). 0 = segmentation off |
| `pre_asr_ms` | ASR time spent *while recording* on pieces whose text was used. Not part of `asr_ms`, which now means ASR time inside `commit` (= the tail) |
| `asr_audio_ms` | (existing) total audio given to ASR after VAD trimming, summed over pieces |

`CommitResult.route.client_ms` (u32, router only): wall time of
`Router::commit` on the device. This is release → result **including the
network** for server results. A server's `total_ms` is only its own commit
time. This is the number to show as "latency" and to log.

Suggested history line (both shells): `total {client_ms ?: total_ms} ms ·
asr {asr_ms} (+{pre_asr_ms} early) · wait {wait_ms} · {segments} seg`.

## 4. Model config (optional, Android with Parakeet only)

`ModelConfig.parakeet_decoding`: `"auto"` (default; omitted from JSON),
`"greedy"` or `"beam"`. `auto` loads a second recognizer (≈ +0.6 GB RSS) so
that decodes without dictionary hotwords run greedy and an empty beam result
is retried greedy. A phone that cannot spare the RAM should send
`"greedy"`. Whisper ignores this field.

## 5. Tests the shells should add

* `CoreJson` / `core_api` round trip: `segment_pause_ms` present when the
  pref is on, absent when off. The three `timings` fields parse when present
  and default to 0 when absent. `route.client_ms` parses.
* `DataLogic` / `prefs`: the hands-free default is 1500, the "Decode while
  speaking" default is on, and an explicit user value is kept.

No server or protocol change is needed by the shells. `HelloOk.features`
(`["segments"]`) is router-internal.
