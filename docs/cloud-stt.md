# Cloud speech-to-text and text cleanup (opt-in, OpenRouter only)

Status: implemented on `feat/or-cloud` (flow-cloud, W1) + `feat/or-route` (core/router/ffi),
decisions **D18** (cloud STT, client-direct, fenced) and **D28** (OpenRouter is the only
provider; second opt-in for cloud text cleanup). Design and the dated OpenRouter API
facts: `docs/proposals/openrouter-only-cloud.md` (§1 facts, §2 design, §4.1 frozen API).
This file is the reference for what is built. The D18 multi-provider design (ElevenLabs,
Deepgram, AssemblyAI, generic OpenAI-compatible, presets, custom base URLs) is
**deleted**; git history keeps it.

**Goal.** In Developer settings the user may attach an OpenRouter API key and opt in,
separately, to (1) cloud speech-to-text and (2) cloud text cleanup. OpenRouter is the only
third party the product can reach, and every OpenRouter model that can do the job is
offered (28 transcription models, ~33 audio-input chat models, ~390 text models; live
catalogue). Fences:

| # | Invariant | Enforced by |
|---|---|---|
| C1 | Off by default; STT never used without `allow_cloud` **and** STT consent for `openrouter.ai` **and** a key **and** `cloud` in `order`; cleanup likewise with `allow_cloud_cleanup`, its **own** consent and `cloud` in `cleanup_order` | `flow-router/tests/policy.rs` (or-route) |
| C2 | Not compiled into client-only builds | `tools/check-build-matrix.sh` (dependency graph) |
| C3 | Keys only in OS secure storage and Rust memory (`SecretString`); never in config JSON, logs, `route_status`, `info_json`, catalogue, crash text or flow-server frames | `flow-contract-tests/tests/no_key_leak.rs` |
| C4 | Calls run on the client; flow-server never sees keys or cloud traffic | §4, `no_key_leak.rs::start_frame_has_no_cloud_fields` |
| C5 | Provider returns raw text; VAD, guard, spoken cleanup, dictionary, LLM step and post-processing run in local flow-core | `equivalence.rs::equivalence_cloud_mock_http` |
| C6 | Silence is never uploaded | `flow-cloud/tests/session.rs::silence_makes_no_request` |
| C7 | Every dictation that used OpenRouter is visible (pill `C`, route row, usage) | shell UI tests |
| **C8** | **One host.** The API root is the constant `https://openrouter.ai/api/v1` (`flow_cloud::BASE_URL`); no config, preset or env var changes it; no redirect is followed; https only; no other provider host string is compiled into a cloud build | `api.rs`, `http.rs`, `contract_openrouter.rs::redirects_are_never_followed`, `check-build-matrix.sh` host scan |

---

## 1. Summary of choices

| Question | Choice |
|---|---|
| Providers | **OpenRouter only** (D28). One adapter, one key, two uses |
| STT path | Per model from the catalogue: **A** `POST /audio/transcriptions` (dedicated ASR, default) or **B** `POST /chat/completions` with an `input_audio` part (chat models, opt-in, sanity-checked). Config override `stt_path: auto\|transcriptions\|chat_audio` |
| STT default | `openai/whisper-large-v3-turbo` (ZDR on Groq + DeepInfra, $0.012-0.04/h) |
| Cleanup default | `meta-llama/llama-3.3-70b-instruct`, `provider.sort: "latency"` (ZDR on Groq / SambaNova) |
| Privacy default | `provider: {zdr: true, data_collection: "deny"}` on every request; no `user`, `session_id`, `HTTP-Referer` or `X-OpenRouter-Title` |
| Where the call runs | Client direct (§4) |
| Batch vs streaming | STT batch at commit; cleanup streamed (abort = stop billing) |
| Encoding | Path A FLAC 16 kHz mono (pure-Rust `flacenc`, lossless, ≈ 50-60 % of WAV), WAV after a format-400 (remembered per model); path B WAV |
| Models | Live catalogue (`/models?output_modalities=all` + `=transcription` + `/endpoints/zdr` + `/models/user` with a key), 24 h cache file, baked offline fallback, free-text ids |
| Cost | Exact `usage.cost`; catalogue estimate when missing, replaced by a deferred `/generation` lookup; `provider.max_price` ceiling on cleanup |

---

## 2. OpenRouter API (facts 2026-10-24)

See `docs/proposals/openrouter-only-cloud.md` §1 (sources, tables, coverage counts,
error table) and the dated snapshots in `docs/proposals/data/`. Summary of what the code
relies on:

* Auth `Authorization: Bearer <key>`. Key info `GET /key`; cost of one generation
  `GET /generation?id=<X-Generation-Id>`.
* Path A body: `{model, input_audio: {data: <base64>, format}, language?, temperature: 0,
  response_format: "json", provider}`; answer `{text, usage: {seconds, cost}}` + header
  `X-Generation-Id`. 60 s upstream timeout (we chunk at 55 s). Biasing only through
  `provider.options.groq.prompt` (documented for Groq whisper).
* Path B: chat `messages` with `{type: "input_audio", input_audio: {data, format}}`.
* Streaming cleanup: SSE; `:` comment lines ignored; mid-stream errors arrive as a `data:`
  event with `error` / `finish_reason: "error"`; usage in the last chunk.
* 503 "no provider meets your routing requirements" = no endpoint satisfies `zdr`/`data_collection`.

---

## 3. Routing

### 3.1 Config and route list

Unchanged from D18 for STT (`order`, `allow_cloud`, `Route::Cloud`, `CloudFactory`,
`set_cloud_key`, `probe_cloud`, `cloud_usage`): see "Changelog for shells" below. D28
(or-route) adds schema 2 (`cloud.openrouter.{privacy,stt,cleanup}`, §8),
`allow_cloud_cleanup`, `cleanup_order`, a cleanup factory and a one-time `migrate_d28`
(proposal §2.1). `order` resolution:

1. `order` empty → legacy D17 policy; **cloud is never implied**.
2. Otherwise walk `order`, skipping routes whose effective flag is off.
3. First eligible route is primary; the rest are the fallback chain.

### 3.2 Failure handling (STT and cleanup)

STT is batch, so OpenRouter is not contacted until commit; the router keeps the PCM
buffer for replay. Mapping (`flow_cloud::http::map_status`, codes = `CloudError::code()`):

| OpenRouter answer | Code | Retry / circuit |
|---|---|---|
| connect / TLS / DNS error | `connect_failed` | 5 s doubling to 60 s |
| 401, 403 (not limit / moderation) | `auth_failed` | sticky until key/config change |
| 402 credits; 403 with `limit_source` (key / guardrail / workspace budget) | `quota_exceeded` | sticky |
| 402 `in_flight_budget_exhausted` | `busy` | one retry if `Retry-After` ≤ 2 s and the deadline allows |
| 403 moderation / guardrail | `provider_error` (detail `moderation`) | not sticky |
| 408 / deadline | `timeout` | 5 s doubling |
| 429 | `busy` | one retry if `Retry-After` ≤ 2 s |
| 502 / 503 (other) / 504 | `provider_error` | one retry (504 also: STT is a pure read) |
| **503 "routing requirements"** | **`no_private_route`** — "model has no zero-retention provider (pick a model marked ZDR, or allow non-ZDR providers)" | sticky until model/privacy change |
| 400/415/422 naming the audio format (path A, FLAC) | retried once as WAV, then `provider_error` | — |
| path B answer fails the sanity check | `provider_error` (detail `chat_audio_unverbatim`) | — |
| 3xx | `provider_error` (`redirect`); never followed | — |
| 2xx with text (even empty) | final | closes |

Error strings carry the status and a sanitised code only (never provider free text, the
key or a URL); a code that contains the key is dropped (`scrub`).

`CloudError::NoPrivateRoute.failure()` is `Failure::NoPrivateRoute`.

### 3.3 Deadline

`commit deadline = now + stt.timeout_base_ms (6000) + 0.5 × audio_ms + llm_timeout_ms`,
capped at 90 s; connect timeout 3 s (STT), 1.5 s (cleanup). Cleanup has its own
`cleanup.timeout_ms` (2500), enforced by flow-core's `run_cleanup`; the stream is aborted
when it fires.

### 3.4 RouteInfo

`RouteInfo.engine = "cloud"`, `provider = "openrouter"`, `model`, `billed_audio_ms`,
`cost_usd`, `cost_estimated`. or-route adds `llm_engine`, `llm_model`, `llm_cost_usd`.

---

## 4. Where the call runs: client direct

Unchanged from D18: keys stay on each device, flow-server keeps `IPAddressDeny=any`, no
protocol change. Cloud cleanup applies to on-device and cloud-STT dictations, **not** to
the server route in v1 (the server uses its own LLM; proposal §2.5).

## 5. Batch vs streaming

STT is batch at commit (`AsrPurpose::Partial` refused, `segmentable() == false`, one
billed call per dictation). OpenRouter's STT endpoint has no streaming. Cleanup streams.

---

## 6. `crates/flow-cloud`

### 6.1 Layout

| File | What |
|---|---|
| `api.rs` | `BASE_URL`, `HOST`, `Api` (client bound to the constant root; https only outside `mock-server`) |
| `http.rs` | pooled blocking reqwest client (rustls + webpki roots, **no redirects**, no Referer), deadline-bounded `send`, OpenRouter error table, retry rules, 8 MB body cap |
| `settings.rs` | `Privacy`, `SttSettings`, `CleanupSettings`, `CleanupScope`, `SttPath(Choice)`, defaults, `valid_model_id` |
| `catalog.rs` | `ModelInfo`, `Catalog`, `CatalogStore` (memory → cache file → baked), `refresh`, `lookup_model`, picker search/rows/badges/price labels |
| `adapters/openrouter.rs` | `OpenRouterStt`: paths A and B, Groq prompt, WAV retry, `verbatim_check`, `TRANSCRIBE_PROMPT` |
| `lib.rs` | `CloudAsr` (chunking, cost), `CloudBackend` (`flow_router::Backend`, incl. `install_cleanup`), router glue (`attach*` = cloud factory + cleanup factory, `factory*`, `cleanup_factory`; `From<&OpenRouterConfig>` for the settings) |
| `cleaner.rs` | `OpenRouterCleaner: flow_core::Cleaner` (SSE, cancel, usage, prefill = `GET /key`) |
| `keyinfo.rs`, `generation.rs`, `budget.rs` | `/key`, deferred `/generation` cost, estimates + `max_price` |
| `encode.rs` | FLAC / WAV / PCM16, VAD-quiet chunking |
| `presets.rs` | `presets()` = exactly one OpenRouter entry (kept for `nativeCloudPresets`) |
| `testing.rs` (`mock-server`) | in-process mock HTTP server (+ SSE streaming, abort counting) and `override_base_url` |
| `data/catalog-baked.json` | trimmed raw OpenRouter JSON (65 models, 28 transcription), generated by `tools/gen-openrouter-catalog.py` from `docs/proposals/data/*-2026-10-24.json`; parsed by the same code as live answers |

Dependencies unchanged: `reqwest` (blocking, json, rustls-tls-webpki-roots), `flacenc`,
`base64`, `serde_json`. No logging crate: flow-cloud never logs.

### 6.2 STT paths

| | A `transcriptions` | B `chat_audio` |
|---|---|---|
| Chosen when | catalogue `output_modalities` has `transcription`; unknown ids | input has `audio`, output `text`, not `:batch` / router |
| Format | FLAC (WAV after a format-400, remembered per model for the process) | WAV |
| Biasing | whisper models: `provider.options.groq.prompt = "Vocabulary: t1, t2."` (≤ 50 terms / 120 words); others none | "Spell these names exactly: …" in the text part |
| Extras | `temperature: 0`, `response_format: "json"`, `language` | `TRANSCRIBE_PROMPT` system message, `max_completion_tokens = ceil(audio_s × 8) + 32`, `reasoning.effort` = `none` or the lowest listed, `stream: false` |
| Guard | full local pipeline (C5) | plus `verbatim_check`: reject words/s > 6, preamble/quote openers, `finish_reason ≠ stop`, text for < 300 ms audio |

Chunks ≤ 55 s at VAD-quiet points; texts joined with one space; costs summed.

### 6.3 Model catalogue

`CatalogStore::new(<cacheDir>/openrouter-catalog.json)`; `current()` never touches the
network; `refresh(key, force)` (≤ 10 s, blocking, off the UI thread) skips a fresh cache
(TTL 24 h) unless forced and keeps the old catalogue on error. Unauthenticated fetches
except `/models/user`. Pagination follows `links.next` on the same origin only. Picker
JSON (`search_json(use, query, filters)`) rows = `ModelInfo` + `recommended`,
`price_label` ("$0.012/h audio", "≈ $0.035/h + output", "≈ $0.0007 per dictation",
"price: see usage" for unknown units) and `badges` (`ZDR`, `fast`, `free (20/min, may
log)`, `chat model, may paraphrase`, `reasoning`, `not allowed by your OpenRouter privacy
settings`). Filters default to ZDR only. `catalog::set_default_store` lets `CloudAsr` /
`OpenRouterCleaner` resolve path and prices from the shell's store (else the baked one).

### 6.4 Cloud text cleanup

`OpenRouterCleaner::new(CleanupSettings, key)`: one streamed `POST /chat/completions`
with `messages = prompt.messages()`, `temperature: 0`, `seed: 0`,
`max_completion_tokens = max(16, 2 × tokens + 32)`, `reasoning.effort` none/lowest (only
when the model lists reasoning), `include_reasoning: false` where supported, `provider:
{zdr, data_collection: "deny", sort, max_price (4 × the highest ZDR endpoint price per
million tokens), allow_fallbacks}`. The SSE reader runs on a helper thread; cancel or
deadline returns within a few ms and drops the connection. Errors carry the `Failure`
code: `CleanError::Unavailable(code)` for fast failures before any output
(`CloudError::is_fast_unavailable`: connect, auth, quota, no private route, busy; a
`RoutedCleaner` may then try the local model), else `CleanError::Backend(code)` (final:
deterministic text). `clean_metered` returns `CleanUsage` (engine `cloud`, model, exact or
estimated cost, generation id, TTFT, latency) even on error or cancel; `engine()`,
`model()`, `timeout_ms()` (= `cleanup.timeout_ms`), `prompt_scope()` and
`estimate_cost_usd()` implement the D28 `Cleaner` additions.
`last_call()` / `info_json()` carry cost, generation id, TTFT, latency, error code; never
text or key. `prefill` only warms the TLS connection with `GET /key` (≤ once per 60 s).
Default scope sends nothing beyond transcript, category/style and dictionary
(`scope()` → `CleanupScope::default()`).

---

## 7. Pipeline and dictionary

`CloudBackend` owns `FlowCore::from_engines(Arc<CloudAsr>, cleaner)`; the whole local
pipeline runs on the provider text (C5). Only `AsrRequest.dictionary_terms` are sent (and
only when `send_dictionary`), never screen words. Dictionary replacements still run
locally; snippet triggers are never sent.

---

## 8. Settings (schema 2, keys never in it)

```json
"allow_cloud": false, "allow_cloud_cleanup": false, "order": [], "cleanup_order": [],
"cloud": { "schema": 2, "openrouter": {
    "privacy": {"zdr": true, "data_collection": "deny"},
    "stt": {"model": "openai/whisper-large-v3-turbo", "stt_path": "auto", "language": "",
            "send_dictionary": true, "format_override": "", "timeout_base_ms": 6000,
            "consent_host": "", "consent_at_ms": 0},
    "cleanup": {"model": "meta-llama/llama-3.3-70b-instruct", "sort": "latency", "timeout_ms": 2500,
                "connect_timeout_ms": 1500, "send_surrounding_text": false, "send_screen_words": false,
                "send_app_name": false, "consent_host": "", "consent_at_ms": 0} },
  "monthly_budget_usd": null }
```

Owned by flow-router (or-route); flow-cloud mirrors the field names in `SttSettings` /
`CleanupSettings` / `Privacy`. There is no base URL, preset, dialect or price field.
Key: Windows Credential Manager `flow-clone/cloud/openrouter`, Android
`SecretVault("flow_cloud_openrouter")`.

---

## 9. Build matrix

| Build | flow-cloud | `cloud_capabilities().adapters` |
|---|---|---|
| Android `local`, Windows `local,app` | **no** | — |
| Android `hybrid` / `remote` (+`cloud`), Windows `…,cloud,app` | yes | `["openrouter"]` |

`tools/check-build-matrix.sh`: the D18 dependency-graph checks, plus (D28) a host-string
scan that fails on any known third-party speech/LLM API host (ElevenLabs, Deepgram,
AssemblyAI, Groq, OpenAI, Anthropic, Mistral, Together, Fireworks, DeepInfra, Cerebras,
SambaNova, Google speech/generative, Speechmatics, Rev, Gladia, xAI, Azure speech,
Cohere, Perplexity) in (a) the Rust sources linked into cloud builds, (b) flow-cloud's
compiled artefact, (c) `FLOW_CLOUD_ARTIFACTS` (APK / .so / .exe paths); flow-cloud's
non-test URL literals must all be `https://openrouter.ai`; the shipped flavours must not
enable flow-cloud's `mock-server` feature. Shell sources are scanned too and count in
`CHECK_BUILD_MATRIX_STRICT_CLOUD=1` (they still hold D18 preset hosts until W4/W5 land).

---

## 10. Tests

| File | What |
|---|---|
| `crates/flow-cloud/src/**` unit tests | settings JSON names, privacy defaults, model-id validation, catalogue parsing (paths, router/batch/free flags, unit sanity, ZDR join, account filter, baked defaults + recommended, labels), budget, verbatim check, SSE parser, error codes |
| `crates/flow-cloud/tests/contract_openrouter.rs` | STT wire contract vs mock OpenRouter: path A/B shapes, base64 round trip, Groq prompt, privacy on/off, chunking, WAV retry once + remembered, path choice per catalogue entry, §3.2 table, transport errors, redirects, deferred cost |
| `crates/flow-cloud/tests/contract_cleaner.rs` | cleanup: request shape (`stream`, `max_price`, `sort`, no `user`/`session_id`), streamed result + usage, reasoning effort, mid-stream error, **cancel < 50 ms + connection dropped**, `run_cleanup` timeout and formatting, JSON fallback, error codes, prefill = `/key` only, deferred cost |
| `crates/flow-cloud/tests/contract_catalog.rs` | refresh (3-4 GETs, key only for `/models/user`), pagination same-origin only, ZDR failure degrades, size cap, cache store, `lookup_model`, `/key`, `/generation` |
| `crates/flow-cloud/tests/session.rs`, `encode.rs` | backend/session behaviour (C5, C6, deadline, probe, router e2e); encodings |
| `crates/flow-cloud/tests/live_smoke.rs` | `FLOW_CLOUD_LIVE_OPENROUTER_KEY` only (else skip lines): STT for the recommended models + one chat-audio model (WER ≤ 10 %), cleanup on 8 messy dictations (guard ≥ 6/8, p90 < timeout), `/key`, live catalogue |
| `crates/flow-contract-tests/**` | equivalence, toggle matrix, segmented, key leak (router, cleaner, catalogue, key info), build matrix incl. the host scan and a planted-host negative test |

Tests reach the mock only through `flow_cloud::testing::override_base_url` (feature
`mock-server`), never through config: configs in tests are production-shaped.

---

## Changelog for shells

**2026-10-24 — D28: OpenRouter only (flow-cloud W1 + contract W6, branch `feat/or-cloud`)**

* **Removed:** the `openai_compat`, `elevenlabs`, `deepgram` and `assemblyai` adapters,
  all presets but one, custom base URLs (and the plain-http LAN path), the dialect and the
  static `price_per_hour` table (`flow_cloud::price_per_hour` is gone). `flow_cloud::presets()`
  returns exactly one OpenRouter entry; `flow_cloud::ADAPTERS == ["openrouter"]`.
* **Base URL is constant.** flow-cloud never reads a base URL from config (schema 2 has
  none; `CloudFactory` / `CleanupFactory` take `&OpenRouterConfig`). `attach` /
  `attach_with` now also install the cleanup factory. Tests that pointed `base_url` at a mock must use
  `flow_cloud::testing::override_base_url(srv.url("/api/v1"))` (guard; serialised;
  `BaseOverride::set` to re-point, e.g. at a closed port). Affects `flow-ffi/tests/{jni_cloud,c_abi}.rs`
  (or-route) and `apps/flow-windows/tests/cloud_integration.rs` (W5).
* **Mock wire format:** STT mocks must answer like OpenRouter (`POST /api/v1/audio/transcriptions`,
  JSON body with base64 `input_audio`; answer `{text, usage{seconds, cost}}`), not
  multipart `file`.
* **New API** (proposal §4.1): `CatalogStore` (`current`, `refresh`, `search_json`,
  `status_json`) for `nativeCloudCatalog` / `nativeRefreshCloudCatalog` / `dev_or_catalog`;
  `keyinfo::fetch` for `nativeCloudKeyInfo` / `dev_or_key_info`; `OpenRouterCleaner` /
  `cleaner_for` for the cleanup factory; `CloudError::code()` (`no_private_route`).
* `CallInfo` gains `stt_path`; `provider_id` is always `"openrouter"`; `error` holds
  `CloudError::code()`.
* `RouteInfo.provider` is `"openrouter"` (was the entry id).


Implementation notes for W and A against the core/router side (branch `cloud-core`,
from `integration` after the three hybrid merges). Every item is additive: shell code
written against the merged D17 router compiles and behaves unchanged; nothing below is
reachable until a shell enables cloud.

**2026-10-22 (review fixes):** cloud keys bound to host + adapter (`set_cloud_key_for`), Test provider needs the typed key for an edited host, `url`-crate base-URL parsing (Kotlin mirror must match `tests/url_parse.rs`), no redirects, http:// by resolved address; details in `docs/hybrid-architecture.md` "Changelog for shells" (review fixes entry).

**2026-10-22 (later) — live route:** while recording, `Router::poll().route` / `start_session_info().route` (JNI `nativePoll` / `nativeStartSessionInfo`) name the cloud provider + model (`{"engine":"cloud","provider":…,"model":…}`), so the pill can say "Transcribing via <provider>…" before the commit; see `docs/hybrid-architecture.md` "Changelog for shells" (live route entry).

**2026-10-22 — flow-core CR, router cloud route, flow-cloud, contract tests landed**

* **Deviations from §3 / §6 (read these first).**
  * `Capabilities` is unchanged (Windows builds it with a struct literal). Cloud support
    is `Router::cloud_capabilities() -> CloudCapabilities { cloud: bool, adapters:
    Vec<String> }` (fixture `cloud_capabilities.json`).
  * `RouterError` gains **no** variant (Windows matches it exhaustively). A config whose
    only listed route is `cloud` and the cloud is unusable (no provider / consent / key,
    or budget used up) gives `NoEngineEnabled`; the reason is in the dead session's
    `commit().route.fallback` (`cloud_not_configured` / `budget_exceeded`) and in the
    error text. A cloud start failure with no other route gives `ServerUnavailable`.
  * Keys: `SecretString` lives in `flow_router` (`flow_router::SecretString`, re-exported
    nowhere else), not in flow-cloud, because `set_cloud_key` / `CloudFactory` need it in
    builds without flow-cloud. No `zeroize` dependency (manual volatile wipe), so the
    Windows lockfile does not change.
  * flow-cloud does not log at all (no `log`/`tracing`); per-call metadata for the dev page
    is `CloudBackend::last_call() -> CallInfo { provider_id, adapter, model, requests,
    bytes, audio_ms, billed_audio_ms, cost_usd, cost_estimated, encode_ms, latency_ms,
    request_id, error, message }` (no key, headers, bodies or text).
  * OpenRouter sends no biasing terms in v1 (§7 `provider.options.<slug>.prompt` not
    wired). Fixtures in `crates/flow-cloud/fixtures/` follow the providers' documented
    shapes; they are not captured responses (no keys on the dev box).
* **flow-core** (CR): `AsrRequest` gains `dictionary_terms`, `screen_terms`, `purpose:
  AsrPurpose { Final, Partial }` (all `Default`; `initial_prompt` unchanged);
  `FlowCore::cleaner() -> Option<Arc<dyn Cleaner>>`; `RouteInfo` gains `model`,
  `billed_audio_ms`, `cost_usd: Option<Usd>`, `cost_estimated` (all omitted from JSON when
  `None`). `flow_core::Usd(pub f64)` is a plain JSON number (newtype only so structs keep
  `Eq`). Code that builds `RouteInfo { .. }` with every field listed needs `..Default::default()`.
* **flow-router** (all `#[serde(default)]`, omitted from JSON while default, so every
  pre-existing fixture is byte-identical):
  * `RouterConfig.allow_cloud: bool`, `.order: Vec<Route>` (`"server"|"client"|"cloud"`),
    `.cloud: CloudConfig { active, providers: Vec<CloudProviderConfig>, monthly_budget_usd:
    Option<Usd> }`; `CloudProviderConfig` = the §8.1 schema exactly (fixture
    `router_config_cloud.json`). `RouterConfig::validate()` now also rejects duplicate
    routes in `order` and runs `CloudConfig::validate()` (§8.1 rules; `http://` only for
    private/tailnet hosts; no credentials in URLs). `CloudProviderConfig::host()`,
    `consent_ok()` (consent is void once the base URL's host changes), `display_name()`.
    `RouterConfig::route_order()` follows `order` when set; `route_allowed(Route)`.
  * `EngineKind::Cloud` (`"cloud"`); `Failure::{CloudNotConfigured, BudgetExceeded,
    QuotaExceeded, ProviderError}` (`cloud_not_configured`, `budget_exceeded`,
    `quota_exceeded`, `provider_error`). Both enums were already `#[non_exhaustive]`.
  * `Router::with_cloud(CloudFactory)`, `with_cloud_adapters(Vec<String>)` (builders; use
    `flow_cloud::attach` instead), `set_cloud_key(id, Option<SecretString>)`,
    `has_cloud_key(id)`, `probe_cloud(&CloudProviderConfig, Option<SecretString>) ->
    HealthReport` (Test provider: transcribes the bundled 3 s JFK clip; `key: None` = the
    stored key; needs no consent; a success for the active provider closes the cloud
    circuit), `cloud_usage() -> CloudUsage`, `set_cloud_usage(CloudUsage)` (restore at
    startup; other months are dropped), `reset_cloud_usage()`, `cloud_capabilities()`.
    `reset_circuit()` resets both circuits. `set_local()` also drops the cached cloud
    backend (so it picks up a newly loaded LLM cleaner).
  * `CloudUsage { month: "YYYY-MM" (UTC), providers: [ProviderUsage { provider_id,
    requests, billed_audio_ms, cost_usd, cost_estimated }] }` (fixture `cloud_usage.json`).
    The router counts only cloud results with billed audio (silence is not a request);
    the shell persists it (`cloud_usage.json` / prefs) after each commit and on exit.
  * `HealthReport` gains `provider`, `model`, `transcript`, `latency_ms`, `cost_usd`
    (fixture `health_cloud_ok.json`); `RouteStatus` gains `cloud_circuit:
    Option<CircuitStatus>` (present only with a cloud factory; fixture
    `route_status_cloud.json`). `commit_result_cloud.json` shows a cloud `route`.
* **Behaviour** (proved by `flow-router/tests/cloud_policy.rs`, 29 tests):
  * C1: the cloud is tried only with a cloud factory **and** `allow_cloud` **and** `cloud`
    in `order` **and** consent for the active provider's current host **and** a key **and**
    budget left. Missing consent/key/provider → route skipped with `cloud_not_configured`
    in `route.fallback`; budget → `budget_exceeded`. `allow_cloud` off or `cloud` not in
    `order` → silently not a route (no fallback reason). `order: []` = legacy `prefer`
    policy, never the cloud.
  * One policy walks the list: the first route that starts is primary; later routes are
    the fallback chain (up to 2). Any failure (start, mid-session, commit) replays 100 %
    of the buffered audio into the next route; a route that failed is not retried in the
    session. `route.fallback` = the primary's failure (later hops keep it).
    `route.provider`/`model`/`cost_*` are set only when the result came from the cloud.
  * Cloud circuit: `auth_failed`, `quota_exceeded` → open until the key (`set_cloud_key`)
    or the active provider config changes; `connect_failed`/`timeout`/`provider_error`
    → 5 s doubling to 60 s; 429 (`busy`) → 5 s. Like the server circuit it is honoured
    only when a later route can serve the session (then `circuit_open`).
  * Cloud commit deadline = `timeout_base_ms + 0.5 × audio + llm_timeout_ms`, ≤ 90 s.
* **flow-cloud** (new crate, link only in cloud flavours): `flow_cloud::attach(router,
  cleaner)` or `attach_with(router, || current_cleaner())` (pass the local core's
  `FlowCore::cleaner()`); `presets() -> Vec<CloudProviderConfig>` (§8.2, no id/consent;
  the Developer "Add provider" list), `price_per_hour(&p)`, `ADAPTERS`, `TEST_CLIP`.
  `CloudBackend` forces `partial_interval_ms = None` (pill: "Transcribing via
  <provider>…", no live preview). Errors are typed (`CloudError` → `Failure`) and carry
  only the HTTP status and a sanitised provider code.
* **Build matrix**: `tools/check-build-matrix.sh` now also requires that flow-core,
  flow-router, flow-ffi `jni,whisper,llm,parakeet` and flow-windows `local,app` contain no
  `flow-cloud|reqwest|hyper|rustls|webpki`, and that flow-cloud contains no ASR/LLM libs.
  Two checks wait for the shells' `cloud` feature and SKIP until then: `flow-ffi
  jni,remote,cloud` and `flow-windows remote,cloud,app` (no ASR/LLM libs). They do not
  fail `CHECK_BUILD_MATRIX_STRICT=1`; run `CHECK_BUILD_MATRIX_STRICT_CLOUD=1` after the
  W/A cloud merges.

### What A needs (flow-ffi + Android)

* `crates/flow-ffi/Cargo.toml`: `cloud = ["dep:flow-cloud"]`, `flow-cloud = { path =
  "../flow-cloud", optional = true }`. Enable it in `tools/build-native.sh --flavor hybrid`
  and `--flavor remote` only (never `local`; the gate above enforces it). flow-cloud uses
  rustls + webpki-roots (no OpenSSL), so the NDK build needs nothing new; `ring` needs
  the NDK clang already used for whisper.
* `new_router`: under `#[cfg(feature = "cloud")]`, `flow_cloud::attach_with(router, move
  || local_core.cleaner())` (the `FlowCore` passed to `flow_open_router`, if any).
* New C ABI (same conventions: JSON strings, `flow_string_free`, never panics):

| C | JNI (`NativeFlowCore`, static) | Semantics |
|---|---|---|
| `int flow_set_cloud_key(FlowHandle*, const char* provider_id, const char* key_or_null)` | `nativeSetCloudKey(h: Long, providerId: String, key: String?): Boolean` | `Router::set_cloud_key`; `null`/empty = forget. Call at startup for every stored provider and on Save/Delete. `-1`/`false` + kind `error` for a core handle. Zero the Java/C copies after the call where possible; never log the arguments |
| `char* flow_probe_cloud(FlowHandle*, const char* provider_json, const char* key_or_null)` | `nativeProbeCloud(h: Long, providerJson: String, key: String?): String` | Blocking `HealthReport` JSON (Test provider) for an unsaved `CloudProviderConfig`; `key` null = stored key. `h = 0` allowed (uses a temporary router: then a key is required). Without feature `cloud`: `ok:false, error:"cloud_not_configured"` |
| `char* flow_cloud_usage(FlowHandle*)` | `nativeCloudUsage(h: Long): String` | `CloudUsage` JSON (persist it after each commit) |
| `int flow_set_cloud_usage(FlowHandle*, const char* usage_json)` | `nativeSetCloudUsage(h: Long, usageJson: String): Boolean` | Restore persisted counters at router open |
| `void flow_reset_cloud_usage(FlowHandle*)` | `nativeResetCloudUsage(h: Long)` | Developer "Reset counters" |
| `char* flow_cloud_capabilities(void)` | `nativeCloudCapabilities(): String` | `CloudCapabilities` JSON of this build (`cloud:false, adapters:[]` without the feature); must match a new `BuildConfig.CAP_CLOUD` |
| `char* flow_cloud_presets(void)` | `nativeCloudPresets(): String` | `presets()` as a JSON array (`[]` without the feature) |

* Existing natives need no signature change: `nativeOpenRouter` / `nativeUpdateRouter`
  accept the cloud fields in the config JSON; `nativeRouteStatus` returns `cloud_circuit`;
  `nativeCommit`'s `route` may be `engine:"cloud"` with `provider`, `model`,
  `billed_audio_ms`, `cost_usd`, `cost_estimated`. New error kind strings on
  `route.fallback`: the four `Failure` values above.
* Kotlin: `DevSettings.cloud` ↔ `RouterConfig.cloud`/`allow_cloud`/`order` (must
  serialise to `router_config_cloud.json` exactly; the existing four fixtures are
  unchanged); `SecretVault("flow_cloud_<id>")`; consent dialog writes `consent_host` =
  host of `base_url` and `consent_at_ms`; changing the base URL host or the adapter clears
  both (the router also refuses a stale consent). Bubble badge `C` when
  `route.engine == "cloud"`. Hide the section when `CloudCapabilities.cloud` is false.

### What W needs (Windows)

* `apps/flow-windows/Cargo.toml`: feature `cloud = ["dep:flow-cloud"]` (default set
  becomes `local,remote,tls,cloud`); `local,app` must stay without it. Update the MSVC
  cross-check loop with a `app,local-engine,remote,cloud` flavour (rustls/ring cross-check
  like flow-remote's `tls`). The Windows `Cargo.lock` gains reqwest/flacenc.
* Router construction: `flow_cloud::attach_with(router, move || adapter.cleaner())` where
  `adapter.cleaner()` returns the loaded core's `FlowCore::cleaner()`; keep calling
  `Router::set_local` after the two-phase LLM load (that rebuilds the cloud backend with
  the cleaner).
* Keys: `CredentialStore` → Credential Manager `flow-clone/cloud/<id>`; at startup call
  `set_cloud_key(id, Some(SecretString::new(secret)))` for each provider; on Save/Delete
  likewise (`None` to forget). Never put keys in `prefs.json`, Tauri events or logs.
* Tauri commands (§8.3) map 1:1: `dev_cloud_test` → `probe_cloud(&form, key_from_form)`,
  `dev_cloud_set_key` / `dev_cloud_clear_key` → `set_cloud_key`, `dev_cloud_usage` →
  `cloud_usage()` (persist to `cloud_usage.json`; restore with `set_cloud_usage` at
  start), plus `dev_cloud_presets` → `flow_cloud::presets()` and
  `cloud_capabilities()` for showing the section.
* Prefs `developer.cloud` + `allow_cloud` + `order` must serialise to
  `router_config_cloud.json` exactly. The engine row: `cloud · <provider> · <model> ·
  $<cost_usd>` (`≈` when `cost_estimated`); overlay badge `C` whenever
  `route.engine == "cloud"`; a cloud failure message from `route.fallback`
  (`auth_failed` → "key rejected", `quota_exceeded` → "out of credits",
  `connect_failed` → "no internet / base URL wrong", `budget_exceeded` → "monthly budget
  reached", `cloud_not_configured` → "set a key and accept the consent").

**2026-10-22 (crates-final):** keyless `custom`-preset `openai_compat` providers on loopback/LAN/Tailscale hosts (no `Authorization` header, consent still required); AssemblyAI create/upload not retried on 504 and still-processing transcripts deleted in the background; budget/C10 routing fix. Details in `docs/hybrid-architecture.md` "Changelog for shells" (remaining review fixes entry).
