# 2026-11-02: "local AI cleanup was dropped" (Android hybrid, 1.0.0)

**Status.** Diagnosed; Fix 1 and Fix 2 merged; owner decision B taken and implemented (D43,
`feat/cleanup-always-runs`). Decisions A (kept off) and C (dictionary) need no code.

**Report.** The owner noticed that the on-device AI cleanup is not running and asked whether it
was always like that, and whether that is why dictation is poor without OpenRouter. The
example had technical terms: the speech model wrote the equivalent of "a UID four" for "a UUID4"
and "pie test" for "pytest", and the final text equalled the raw transcript. Yesterday's log of
six dictations showed `route=local`, `engine=sherpa_online`, five cleaned by OpenRouter
`llama-3.3-70b`, one "skipped cleanup". (Synthetic stand-ins only; the real text stays off the
repo.)

**Answer, short.**

1. **The on-device LLM never ran on this phone because it is off by default.** "Clean up on this
   phone" (`Settings.llmEnabled`, pref `llm_enabled`) has defaulted to `false` since the first
   Android settings commit (`aad4afc`, 2026-10-20). No migration has ever switched it on or off.
   The 2026-10-30 settings redesign (`24c8c4d`) moved the switch from a Settings toggle onto the
   new AI cleanup page's engine list. That is probably why it now *looks* dropped. Not a
   regression.
2. **Without OpenRouter, a phone whose AI is off gets the fixed rules only** (D27: punctuation,
   fillers, numbers, lists). No AI is involved, so "a UID four" stays as it is. This is by
   design (D27, D36 "upgrading never turns cleanup on").
3. **A real bug, from D36 (`ad1dd4f`, 2026-10-30, in v0.11.0), explains the one "skipped"
   dictation.** The default cleanup order is local → server → cloud. With the phone AI off,
   the chain becomes server → cloud. Because the cloud was not the *first* hop, the core did
   not take the D30 path (raw transcript to the cloud, no skip). It ran the on-device
   pipeline instead, including D27's "already clean" skip (`llm_skip_clean`, tuned for the
   1.5B). On a tidy-looking transcript the skip stops the **whole** chain, so OpenRouter never
   saw it (`llm=llm_skipped_clean`, "AI not needed (already clean)"). On the other dictations
   the server hop refused (the phone was on `route=local`, server not reachable), and the cloud
   cleaned the *rules'* output with prompt v2 and the strict guard instead of D30's raw-in v4c.
   **Fixed** (below) for the case where the server is known to be unusable.
4. **No AI would have fixed "UID four" → "UUID4" anyway.** The guard rejects any word the
   speech did not back (D13/D30). A cloud answer "Generate a UUID4 with pytest." is
   discarded as `llm_unsupported`, and the rules' text is pasted. This is pinned by
   `an_unreachable_server_before_the_cloud_does_not_skip_ai_cleanup`. **The fix for jargon is
   the personal dictionary**: a replacement `UID four → UUID4` / `pie test → pytest`. It is
   applied deterministically in post-processing on every route, with or without AI (pinned by
   `a_dictionary_replacement_fixes_misheard_terms_without_any_ai`). The replacement's target
   is also sent to the speech model as a hotword (sherpa/Parakeet re-ranking, whisper prompt).
5. **Silent gap in the log, fixed.** When no AI step ran at all, the debug line had `llm: null`
   and nothing saying why. A new Android field, `llm_off`, now gives the reason.

## Exact conditions for on-device cleanup (Android hybrid)

All of the following must hold, in this order:

| # | Gate | Where | If it fails |
|---|---|---|---|
| 1 | "Clean up on this phone" on (`llm_enabled`, **default off**) and the local entry not switched off in the cleanup order (`cleanup_off`) | `CleanupEngines.on`, `Settings.effectiveCleanup` | the session gets `cleanup_level: none` + `raw_mode` (unless OpenRouter/server cleanup is on) → no LLM, rules only; log `llm: null`, now `llm_off: ai_cleanup_off` |
| 2 | A GGUF is imported and loads (`EnginePlan.LlmStatus.Loaded`) | `EngineLogic.kt`, `load_models` | no local hop; `llm_off: no_phone_model` / `phone_model_failed` |
| 3 | Not Command Mode, not a terminal app, not a whole-utterance snippet | `DictationController` (`rawMode`), `cleanup::should_run`, `engine.rs` | rules only |
| 4 | Local is the first *ready* engine in the cleanup order (default local → server → cloud) | router `plan`, `ChainCleaner` | a ready server / cloud ahead of it cleans first; local only runs if those fail, time out or are refused |
| 5 | Not "already clean" (`llm_skip_clean`, default on; since D43 only when the on-device model would clean first): skipped when the rules' text is ≤ 20 words, one line, capitalised, ends in punctuation, no filler/command words, no repeated word, no 15+-word sentence without a comma, < 3 commas (email always runs) | `cleanup::llm_can_help` | `llm: llm_skipped_clean` ("AI not needed") |
| 6 | Answers within the budget (`ANDROID_LLM_TIMEOUT_MS` = 3000) | `run_cleanup_metered_for` | `llm_timeout`, rules' text |
| 7 | Answer passes the strict guard (no new words, numbers, dropped content) | `guard::check` | `llm_unsupported` / other reason, rules' text |

There are **no** battery, thermal, length-cap or circuit-breaker gates on the local LLM, and no
language gate. A Hebrew dictation runs the LLM with a language line; only the English rule layer
is skipped (D33). Gate 5 is the one most likely to hide misheard terms: a mis-transcribed but
grammatical sentence looks "clean".

## Fix 1: cloud-first in practice (`crates/flow-core/src/cleanup.rs`, `crates/flow-router/src/server_clean.rs`)

* `Cleaner::ready()` (new, default = `available()`): "would a call be tried now rather than
  refused up front". `ServerClean` overrides it with its existing `ready()`: circuit closed
  and not known to lack an LLM.
* `ChainCleaner::ai_parts` now finds the first hop that is cloud, on-device, or ready. It
  passes over leading server hops that would refuse. If that hop is the cloud, D30 applies:
  raw transcript, v4c, loose guard, no skip, with the hops after it as the fallback. An
  on-device hop is never passed over, and a cloud hop is never passed over (its refusals keep
  showing as `cloud_fallback`).
* Tests: `flow-core/tests/d36_server_cleanup.rs::{ai_parts_skips_leading_hops_that_would_refuse,
  an_unreachable_server_before_the_cloud_does_not_skip_ai_cleanup}` (both red before) and
  `flow-router/tests/d36_cleanup_order.rs::unreachable_server_before_the_cloud_leaves_the_cloud_first`
  (real router: phone AI off, order L/S/C, server disconnected).

**Limit (removed by D43).** Until the server hop has failed once, its circuit is closed and it
counts as ready, so the *first* tidy dictation after the server dropped was still skipped
(`llm_skipped_clean`). Since D43 the skip is only evaluated when the on-device model would clean
first, so that dictation goes to the server, which fails, and then to the cloud. Pinned by
`flow-router/tests/d36_cleanup_order.rs::unreachable_server_before_the_cloud_leaves_the_cloud_first`
and `flow-core/tests/d36_server_cleanup.rs::tidy_text_after_a_server_drop_goes_to_the_cloud_not_the_local_model`.

## Fix 2: `llm_off` on the debug line (Android)

`LlmOutcome.offReason` (pure function) and `FlowService.llmOffReason` add `llm_off` to a
dictation line that has a transcript, no AI step (`llm: null`) and is not Command Mode.
Values: `ai_cleanup_off` (no engine on), `no_phone_model`, `phone_model_failed`,
`no_engine_available` (an engine is on but none could run: terminal app, snippet, server without
an LLM, …). The in-app viewer shows "no AI cleanup (…)". Tests: `WritingStyleTest.llmOffReasonSaysWhyNoAiCleanupRan`,
`DebugLogTest.dictationLineSaysWhyNoAiCleanupRan`. Windows has no such field yet.

## Debug fields that explain a dictation's cleanup

`llm` (`formatted` | `llm_skipped_clean` | `llm_timeout` | `llm_error: <code>` | null),
`llm_off` (new), `llm_engine` (local | server | cloud: who answered), `llm_model`,
`cleanup_route` + `cloud_fallback` + `rejected_text` + `guard_detail` (only when the D30 cloud
path was tried). Before Fix 1, a cloud answer reached through the chain (server → cloud) had
`llm_engine: cloud` but **no** `cleanup_route`. That is how to tell the bypass apart in old logs.

## Owner decisions

* **A. Phone AI default.** Keep "Clean up on this phone" off by default, or turn it on when a
  model is imported? The 1.5B helps on messy speech (D27: rules 89 % → 92 %) but will not fix
  jargon (gate 7).
* **B. Skip with an off-phone engine. Decided 2026-11-02 (D43): no.** `llm_skip_clean` now
  applies only when the on-device model would clean first. A server or cloud cleaner always gets
  the dictation (guard and time limits unchanged), the server's own sessions included, and a tidy
  dictation never falls through to the on-device model. Cost: one LLM call per tidy dictation.
* **A** stays as is: the phone AI is off by default.
* **C. Jargon.** Add the owner's terms to the dictionary as replacements (no code change).
  Optionally, a "suggest a replacement" path from Learn-from-edits.

## Device facts that would confirm the diagnosis

1. Settings: `llm_enabled`, and whether an LLM GGUF path is set (AI cleanup page: phone row
   text).
2. Dev store: `cleanup_order`, `cleanup_off`, `allow_cloud_cleanup`, and whether a server host
   is configured.
3. The "skipped" dictation's debug line: `llm`, `llm_engine`, `cleanup_route`, `route_order`,
   `fallback`.
