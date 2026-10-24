# OpenRouter-only cloud: STT + text cleanup

Status: design proposal (branch `docs/openrouter-design`, off `integration` v0.3.0).
Draft decision **D28** (§7). Supersedes parts of D18 / `docs/cloud-stt.md` and fills in
D27 item 7 ("cloud text cleanup is a separate opt-in, not built; awaiting user").

**User decision (2026-10-24):** "Regarding the opt-in cloud text cleanup, let's also make
it an option. But, regarding that and the optional cloud STT provider, it's important to
me that the only provider being used is OpenRouter, and that we support as many models as
possible from within OpenRouter."

So:

1. OpenRouter is the **only** third party the product can talk to, for both STT and text
   cleanup. No other adapter can be reached from the UI. Better still, none is compiled in.
2. Model coverage means **every OpenRouter model that can do the job**: every
   transcription model, every audio-input chat model (as a second STT path), and every
   text-output chat model for cleanup. The catalogue comes from OpenRouter at runtime.
3. Cloud text cleanup is a **second, separate opt-in**. Turning it on sends text to
   OpenRouter. Voice is sent only if cloud STT is also on.

All API facts below were retrieved on **2026-10-24** from OpenRouter's own docs and live
JSON. Snapshots are in `docs/proposals/data/`:

| File | Source |
|---|---|
| `openrouter-models-2026-10-24.json` | `GET https://openrouter.ai/api/v1/models` (default = text output; 469 models) |
| `openrouter-models-transcription-2026-10-24.json` | `GET https://openrouter.ai/api/v1/models?output_modalities=transcription` (28 models) |
| `openrouter-endpoints-zdr-2026-10-24.json` | `GET https://openrouter.ai/api/v1/endpoints/zdr` (1072 ZDR endpoints) |

---

## 1. OpenRouter API facts (2026-10-24)

### 1.1 Speech-to-text, path A: dedicated transcription endpoint

Source: <https://openrouter.ai/docs/guides/overview/multimodal/stt> (also as `.md`).

| Item | Fact |
|---|---|
| Endpoint | `POST https://openrouter.ai/api/v1/audio/transcriptions`, same auth as chat (`Authorization: Bearer <key>`) |
| JSON body | `model` (required); `input_audio: {data: <raw base64, not a data URI>, format}` (required); `language` (ISO-639-1, auto if omitted); `temperature` 0-1; `response_format` `json` (default) \| `verbose_json`; `timestamp_granularities` (`segment`/`word`, verbose only); `provider: {zdr, data_collection, options: {<provider slug>: {…}}}` |
| Multipart | OpenAI-compatible (`file`, `model`, `language`, `temperature`, `response_format`, `timestamp_granularities`). `prompt` is **accepted but ignored**. `text`/`srt`/`vtt` → 400 |
| Formats | "vary by provider": wav, mp3, flac, m4a, ogg, webm, aac |
| Size | 25 MB inline (multipart cap). Larger base64 only for OpenAI and Groq models, or via `input_audio.url` where the model accepts a URL. "About 13 minutes of 16 kHz mono WAV" |
| Timeout | **60 s upstream timeout per request**. Split longer recordings |
| Biasing | Only through `provider.options.<slug>` (doc example: `"groq": {"prompt": "Expected vocabulary: …"}`). Only the serving provider's options are forwarded. Some providers drop unknown fields silently (e.g. Deepgram) |
| Routing | **`order`, `only` and `ignore` are not applied to transcription requests.** `zdr` is applied. `data_collection` is listed as a data-policy preference with no more detail |
| Response | `{text, usage: {seconds, total_tokens, input_tokens, output_tokens, cost}}`, `cost` in USD. Header `X-Generation-Id`. `verbose_json` adds `language`, `duration`, `confidence`, `segments`, `words` where the provider supports them |
| Pricing | Duration-based models are priced "per second of audio input". Token-based models (e.g. `openai/gpt-4o-transcribe`) are priced per input/output token |
| Streaming | Not mentioned (assume none) |
| Discovery | `GET /api/v1/models?output_modalities=transcription` |

**Coverage today: 28 transcription models. 21 have at least one ZDR endpoint.** The 7
with no ZDR endpoint are `openai/gpt-transcribe`, `openai/gpt-4o-transcribe`,
`openai/gpt-4o-mini-transcribe`, `openai/whisper-1`, `google/gemini-3.5-transcribe`,
`x-ai/grok-stt-1.0` and `meta/muse-voice-transcribe-1.0`.

Snapshot prices are USD per second of audio (`pricing.prompt`). The ×3600 column is the
price per hour.

| Model | $/s | ≈ $/h | ZDR endpoints (tag) |
|---|---|---|---|
| `openai/whisper-large-v3-turbo` | 0.00000333 (DeepInfra) / 0.0000111 (Groq) | 0.012 / 0.04 | `deepinfra/us`, `groq` |
| `qwen/qwen3-asr-0.6b` | 0.00000333 | 0.012 | `deepinfra/us` |
| `nvidia/nemotron-3.5-asr-streaming-multilingual-0.6b` | 0.00000333 | 0.012 | `deepinfra/us` |
| `openai/whisper-large-v3` | 0.0000075 | 0.027 | DeepInfra, Together, Groq |
| `qwen/qwen3-asr-1.7b` | 0.0000075 | 0.027 | `deepinfra/us` |
| `mistralai/voxtral-mini-3b-2507` | 0.0000167 | 0.06 | `deepinfra/bf16` |
| `nvidia/parakeet-tdt-0.6b-v3` | 0.000025 | 0.09 | `together` |
| `mistralai/voxtral-mini-transcribe-3-0` | 0.0000583 | 0.21 | `mistral/zdr`, `/eu`, `/us` |
| `elevenlabs/scribe-v2` | 0.0000611 | 0.22 | `elevenlabs` |
| `deepgram/nova-3` | 0.0000717 | 0.26 | `deepgram` |
| `assemblyai/universal-3-6-pro` | 0.0000833 | 0.30 | `assemblyai` |
| `openai/gpt-transcribe` | 0.000075 | 0.27 | **none** |

Units we could not confirm: `microsoft/mai-transcribe-2` lists `prompt: "0.1"` and
`mai-transcribe-1.5` lists `"0.36"`. That would be absurd per second; it looks per hour or
per minute. The app treats any transcription price above $0.01/s as "unknown unit". It
shows "price: see usage", and the budget relies on `usage.cost` (§3.5).

> **Finding: the shipped default is probably broken.** On `integration`, the
> `openrouter` preset defaults to `openai/gpt-transcribe` with `zero_retention: true`.
> The adapter then sends `provider: {zdr: true, data_collection: "deny"}`. That model has
> no ZDR endpoint in `/api/v1/endpoints/zdr`. Per the provider-routing docs, a request
> whose routing requirements no provider meets fails, most likely with HTTP 503 ("no
> available model provider that meets your routing requirements",
> <https://openrouter.ai/docs/api_reference/errors-and-debugging>). This is unconfirmed:
> no key was available. The D28 default (§2.3) fixes it either way.

### 1.2 Speech-to-text, path B: chat completions with audio input

Sources: <https://openrouter.ai/docs/guides/overview/multimodal/audio> and the chat
OpenAPI page
<https://openrouter.ai/docs/api/api-reference/chat/create-a-chat-completion> (schema
`ChatContentAudio`).

| Item | Fact |
|---|---|
| Endpoint | `POST https://openrouter.ai/api/v1/chat/completions` |
| Content part | `{"type": "input_audio", "input_audio": {"data": "<base64>", "format": "wav"}}`. `data` and `format` are required |
| Encoding | Base64 only: "direct URLs are not supported for audio content" |
| Formats | "vary by provider": wav, mp3, aiff, aac, ogg, flac, m4a, pcm16, pcm24 |
| Size / timeout | Not documented for chat audio |
| Pricing | `pricing.audio` = "Price in USD per audio input token" (models OpenAPI). Tokens per second of audio are not documented by OpenRouter. Google documents 32 tokens/s for Gemini (`docs/cloud-stt.md` §2.2, <https://ai.google.dev/gemini-api/docs/audio>) |
| Discovery | `/api/v1/models` with `architecture.input_modalities` containing `"audio"` |

**Coverage today: 45 models list audio input. 33 remain after dropping 12 `:batch`
entries (Batch API only) and the `openrouter/auto*` routers, which have price `-1`.** Of
those 33, **18 have a ZDR endpoint**: mostly Gemini on Vertex, plus `xiaomi/mimo-v2.6-*`,
`mistralai/voxtral-small-24b-2507`, `thinkingmachines/inkling*` and
`perceptron/perceptron-mk1.5`. Three are `:free`.

Example, `google/gemini-3.5-flash-lite`: 0.0000003 $/audio token at 32 tokens/s comes to
≈ $0.035 per hour of audio, plus output tokens. Chat models are a cheap way in, but they
are **not verbatim ASR**. D18 rejected them because they may answer, translate or
summarise. §2.4 says how D28 admits them anyway, with fences, because the user wants
maximum model coverage.

### 1.3 Text cleanup: chat completions

Sources: chat OpenAPI page (above),
<https://openrouter.ai/docs/guides/routing/provider-selection>,
<https://openrouter.ai/docs/api_reference/streaming>,
<https://openrouter.ai/docs/guides/best-practices/latency-and-performance>,
<https://openrouter.ai/docs/guides/features/service-tiers>,
<https://openrouter.ai/docs/guides/routing/model-variants/overview>.

| Item | Fact |
|---|---|
| Request | `messages` (system/user/assistant), `model`, `temperature` (0-2), `max_completion_tokens` (`max_tokens` deprecated; "some providers enforce a minimum of 16"), `stop` (≤ 4), `seed`, `stream`, `stream_options`, `reasoning: {effort: max\|xhigh\|high\|medium\|low\|minimal\|none}`, `service_tier` (`auto\|default\|flex\|priority\|fast\|scale\|ultrafast`), `session_id` (sticky routing key for prompt-cache hits, ≤ 256 chars), `models[]` (model fallback on 5xx / rate limit), `provider` |
| `provider` object | `order[]`, `allow_fallbacks` (default true), `require_parameters` (default false), `data_collection` `allow`(default)\|`deny`, `zdr` bool, `enforce_distillable_text`, `only[]`, `ignore[]`, `quantizations[]`, `sort` `price\|throughput\|latency` or `{by, partition}`, `max_price {prompt, completion, request, image, audio}` (USD per million tokens for prompt/completion), `preferred_min_throughput` / `preferred_max_latency` (soft; p50/p75/p90/p99 over a rolling 5-min window), `options` |
| Defaults | Without `order`/`sort`, OpenRouter load-balances by price, weighted by inverse square of price, avoiding recent outages. Setting `sort` disables load balancing. `:nitro` = sort by throughput + admits priority-tier endpoints. `:floor` = sort by price + flex endpoints |
| Response | `choices[].message.content`, `finish_reason` (`stop\|length\|content_filter\|error\|tool_calls`), `usage {prompt_tokens, completion_tokens, prompt_tokens_details.cached_tokens, completion_tokens_details.reasoning_tokens, cost, is_byok, cost_details}`. Usage always comes back on non-streaming requests. On streaming it arrives in the final chunk before `[DONE]` |
| Streaming | SSE. Ignore comment lines such as `: OPENROUTER PROCESSING`. Errors before the stream starts are plain JSON with an HTTP status. Mid-stream errors arrive as a `data:` event with top-level `error` and `finish_reason: "error"` under HTTP 200. **Aborting the connection stops processing and billing for supported providers**; non-streaming requests are billed in full |
| Router metadata | Opt-in header `X-OpenRouter-Metadata: enabled` adds `openrouter_metadata` (strategy, summary, …) to responses (<https://openrouter.ai/docs/guides/features/router-metadata>) |
| Coverage | 469 text-output models. 387 remain without `:batch` and routers. **269 have a ZDR endpoint** |

### 1.4 Privacy controls

Sources: <https://openrouter.ai/docs/guides/features/zdr>,
<https://openrouter.ai/docs/guides/privacy/data-collection>,
<https://openrouter.ai/docs/guides/privacy/provider-logging>, provider-selection (above).

* OpenRouter itself stores **no prompts or responses** unless the account opts in to
  Input & Output Logging or to "OpenRouter use of inputs/outputs". Both are off by
  default. It does store request **metadata** (token counts, latency, …).
* `provider.data_collection: "deny"` means "use only providers which do not collect user
  data". If no provider qualifies, the request errors.
* `provider.zdr: true` routes only to Zero-Data-Retention endpoints. It is an OR with the
  account and guardrail settings: it can enforce ZDR but never disable it. If OpenRouter
  can't establish an endpoint's policy, it assumes the endpoint "retains and trains".
  Implicit in-memory provider prompt caching is not counted as retention.
* ZDR endpoint list: `GET https://openrouter.ai/api/v1/endpoints/zdr`. It needs no key
  and is updated automatically.
* Account-level privacy settings (training opt-out, ZDR per model group) also apply.
  `GET /api/v1/models/user` (authenticated) lists models "filtered by user provider
  preferences, privacy settings, and guardrails"
  (<https://openrouter.ai/docs/api/api-reference/models/list-models-filtered-by-user-provider-preferences-privacy-settings-and-guardrails>).
* In-region hosts `eu.openrouter.ai` / `us.openrouter.ai` exist on Business/Enterprise
  plans only.

**Most private routing (the D28 default):** `provider: {"zdr": true, "data_collection":
"deny"}`. Leave `allow_fallbacks` at its default: fallbacks still honour the ZDR filter.
Send no `user` field. Send no attribution headers (§1.5).

### 1.5 Auth, key info, limits, errors

Sources: <https://openrouter.ai/docs/api/reference/authentication>,
<https://openrouter.ai/docs/api/reference/limits>,
<https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key>,
<https://openrouter.ai/docs/api_reference/errors-and-debugging>,
<https://openrouter.ai/docs/api/api-reference/generations/get-request-&-usage-metadata-for-a-generation>,
<https://openrouter.ai/docs/guides/overview/auth/oauth>.

| Item | Fact |
|---|---|
| Auth | `Authorization: Bearer <OPENROUTER_API_KEY>` |
| Attribution headers (optional) | `HTTP-Referer` (site URL) and `X-OpenRouter-Title` (app name), used for openrouter.ai rankings. The current page does not mention the older `X-Title` (unconfirmed whether it still works). **D28 sends neither**: they only publish usage |
| Key info | `GET https://openrouter.ai/api/v1/key` → `data: {label, limit, limit_reset, limit_remaining, include_byok_in_limit, budgets, effective_budget, usage, usage_daily, usage_weekly, usage_monthly, byok_usage*, is_free_tier, free_model_daily_requests {used, limit, remaining}, rate_limit (deprecated)}`. Returns 401 without a key (checked) |
| Account credits | `GET /api/v1/credits` needs a **management key**, so it is not used. `limit_remaining` / `usage_monthly` from `/key` are what the app shows |
| Per-generation cost | `GET /api/v1/generation?id=<X-Generation-Id>` → `total_cost`, `usage`, `native_tokens_*`, `cancelled` |
| Rate limits | `:free` models: 20 req/min, and 50 req/day (< 10 credits ever bought) or 1000/day. Paid models: no platform cap stated; per-model limits differ. Extra keys don't raise limits. Cloudflare DDoS protection |
| Errors | 400 bad params; 401 bad key; 402 insufficient credits (`in_flight_budget_exhausted` is transient + `Retry-After`; `weight_exceeds_budget` is not); 403 forbidden / guardrail / moderation / **budget** (`limit_source` `openrouter_key_limit` \| `openrouter_guardrail_budget` \| `openrouter_workspace_budget`, `resets_at`, no `Retry-After`); 408 timeout; 429 rate limited (`Retry-After` sometimes); 502 model down / bad upstream response; **503 no provider meets your routing requirements** |
| Empty output | 200 with no content can happen (cold start, scaling, reasoning ate `max_tokens`). Prompt processing may still be charged |
| Key safety | OpenRouter recommends a credit limit on every key. It is a GitHub secret-scanning partner |
| OAuth PKCE | `https://openrouter.ai/auth?callback_url=…&code_challenge=…&code_challenge_method=S256`. Localhost callbacks are allowed on any port. Possible later "Connect OpenRouter" button (§2.6); not in v1 |

### 1.6 Pricing fields (`/api/v1/models`)

Source: <https://openrouter.ai/docs/guides/overview/models> and the models OpenAPI page
<https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties>.

* Every price is a **string of USD per unit**. `"0"` means free.
* Units: `prompt` per input token, `completion` per output token, `request` per request,
  `image` per input image, `audio` per audio input token, `input_audio_cache` per cached
  audio token, `internal_reasoning` per reasoning token, `input_cache_read` /
  `input_cache_write` per cached token, `web_search` per search.
* `overrides[]` hold conditional prices: `min_prompt_tokens`, a UTC time window, or
  `utc_days`.
* `-1` appears on routers (`openrouter/auto`, …) and means "variable"; this is inferred,
  not documented.
* For transcription models, `prompt` is the per-second price. This is inferred: the STT
  doc says duration-priced models are "priced per second", and gpt-transcribe's 0.000075
  matches OpenAI's $0.0045/min.
* The model-level price is the **cheapest** endpoint's. A ZDR endpoint can cost more:
  `whisper-large-v3-turbo` is 0.00000333 on DeepInfra but 0.0000111 on Groq. Per-endpoint
  prices come from `GET /api/v1/models/{author}/{slug}/endpoints`, which also has
  `latency_last_30m`, `throughput_last_30m` and `uptime_*`. Latency and throughput were
  `null` for every STT endpoint we fetched unauthenticated.
* Query params: `output_modalities` (comma list or `all`), `supported_parameters`,
  `sort` (`pricing-low-to-high`, `throughput-high-to-low`, `latency-low-to-high`, …),
  `offset`/`limit` (default 500, max 1000).
* `GET /api/v1/models/count` returns counts.

---

## 2. Design

### 2.1 What changes: OpenRouter is the only cloud provider

| Today (D18) | D28 |
|---|---|
| 5 adapters: `openai_compat`, `openrouter`, `elevenlabs`, `deepgram`, `assemblyai` | **One adapter: `openrouter`.** The other four files are deleted from `crates/flow-cloud/src/adapters/` with their fixtures and contract tests. Git history keeps them. `flow_router::cloud::ADAPTERS = ["openrouter"]` |
| 7 presets incl. `custom` (user base URL, keyless LAN) | **No presets list.** One fixed provider config. `base_url` is a constant `https://openrouter.ai/api/v1`, not user-editable, and `validate()` rejects any other value |
| Up to 8 providers, `active` id | Exactly one OpenRouter account entry (`id = "openrouter"`) with two **uses**: `stt` and `cleanup`, each with its own model and opt-in |
| Static `price_per_hour` table | Prices from the OpenRouter catalogue (§2.2) + `usage.cost` |

**Recommendation: delete, don't hide.** The user wants nothing else used, and "ideally
not compiled in". Hiding adapters behind UI flags leaves reachable code paths: a
hand-edited `prefs.json`, an imported settings file, or an old config with `adapter:
"elevenlabs"`. Deleting the adapters makes "only OpenRouter" a property of the binary.

A new build-matrix check backs this up. `tools/check-build-matrix.sh` greps the cloud
builds' `.so`/`.exe` strings for `api.elevenlabs.io|api.deepgram.com|api.assemblyai.com|api.groq.com|api.openai.com`
and fails on any hit. A unit test asserts `ADAPTERS.len() == 1` and that `validate()`
refuses every host except `openrouter.ai`. Self-hosted STT servers (speaches/LocalAI via
`custom`) go too. The user's own `flow-server` remains the self-hosted path, so this is
no loss in practice.

**Migration (one-time, on first load of a D28 build; Rust `CloudConfig::migrate_d28`,
mirrored in both shells' key stores):**

1. Take the first provider with `adapter == "openrouter"`. Keep its key, `model`,
   `language`, `send_dictionary` and consent. Consent stays valid because the host is
   unchanged (`openrouter.ai`). Rename its id to `openrouter`, and move its key
   (Credential Manager `flow-clone/cloud/<old-id>` → `flow-clone/cloud/openrouter`;
   Android `SecretVault("flow_cloud_<old-id>")` → `flow_cloud_openrouter`).
2. Delete every other provider entry **and its stored key** (Credential Manager entry /
   Keystore ciphertext). Delete their usage counters. Keep the month's cost total, so the
   monthly budget stays honest.
3. If the active provider was not OpenRouter, do not move it to OpenRouter silently.
   Remove `cloud` from `order`, set `allow_cloud = false`, and show a one-time notice:
   "Cloud speech-to-text now uses OpenRouter only. <Provider> was removed and its key
   deleted. Set up OpenRouter to keep using cloud STT."
4. If the migrated model has no ZDR endpoint and ZDR is on (the `gpt-transcribe` case,
   §1.1), switch the model to the new default and say so in the same notice. A user who
   turned ZDR off keeps their model.
5. Set `cloud.schema = 2`. Migration runs once and is idempotent. It is covered by
   fixtures for every old preset.

### 2.2 Model catalogue

New module `crates/flow-cloud/src/catalog.rs`. Rust owns it so both shells show identical
lists.

```rust
pub struct ModelInfo {            // trimmed from /api/v1/models (+ zdr list)
    pub id: String, pub name: String, pub context_length: u32,
    pub input: Modalities, pub output: Modalities,          // bitflags: text|audio|image|transcription…
    pub stt_path: Option<SttPath>,                           // Transcriptions | ChatAudio | None
    pub price: Price,             // per_audio_second | per_audio_token | prompt | completion | request (USD, f64)
    pub price_unit_ok: bool,      // false for the mai-transcribe-style outliers (§1.1)
    pub zdr: bool,                // model_id appears in /api/v1/endpoints/zdr
    pub fast_zdr_providers: Vec<String>, // ZDR endpoints on groq|cerebras|sambanova… (latency hint)
    pub reasoning_mandatory: bool, pub supports: ParamSet,  // temperature, max_tokens, reasoning, stop…
    pub free: bool, pub batch_only: bool, pub router: bool, // :free, :batch, price -1
}
pub struct Catalog { pub fetched_at_ms: u64, pub source: CatalogSource /*Live|Cache|Baked*/, pub models: Vec<ModelInfo> }
impl Catalog {
    pub fn stt_models(&self) -> impl Iterator<Item=&ModelInfo>;     // stt_path.is_some(), !batch_only, !router
    pub fn cleanup_models(&self) -> impl Iterator<Item=&ModelInfo>; // text in & text out, !batch_only, !router
    pub fn search(&self, q: &str, use_: Use, filters: Filters) -> Vec<&ModelInfo>;
}
pub fn refresh(http: &Client, key: Option<&SecretString>) -> Result<Catalog, CloudError>;
```

* **Fetch:** three unauthenticated GETs, in parallel, under one deadline (10 s):
  * `/api/v1/models?output_modalities=all` (all kinds, one call). Use `limit=1000`. If
    `links.next` is present, follow it, same host only.
  * `/api/v1/endpoints/zdr`.
  * When a key is stored, also `/api/v1/models/user?output_modalities=all`, which
    reflects the account's own privacy settings. Models absent from it are shown as "not
    allowed by your OpenRouter privacy settings".
  * Each response is capped at 8 MB. The JSON is parsed leniently: unknown fields are
    ignored and missing fields default.
* **When:** only after the user opened the OpenRouter page, or while a cloud use is
  enabled. Never in client-only builds, and never in the background while both uses are
  off. Fetching the catalogue sends no user data, but it does reveal the IP to
  openrouter.ai, which is the same host the user consented to.
* **Cache:** a trimmed catalogue (≈ 100-150 KB) goes in the app cache dir as
  `openrouter-catalog.json` (Windows `%LOCALAPPDATA%\FlowClone\cache\`, Android
  `cacheDir`). TTL is 24 h. A stale cache is used at once while a refresh runs. Manual
  "Refresh models". `If-None-Match`/ETag support is unconfirmed, so v1 sends no
  conditional request.
* **Offline fallback:** `crates/flow-cloud/data/catalog-baked.json` is generated from the
  dated snapshot by `tools/gen-openrouter-catalog.py`, the probe promoted with that
  commit. It holds the recommended defaults plus about 12 STT and 12 cleanup models. It
  is used when there is no cache and no network, and is tagged `source: Baked` with its
  date.
* **Free-text model id:** always allowed, e.g. a model too new for the cache, or a
  `:nitro` variant. Those variants are valid request models but are not listed
  (variants doc). Validation is only `^[a-z0-9._~/-]+(:[a-z]+)*$` ≤ 128 chars. Unknown
  ids get `stt_path` from a cheap `GET /api/v1/model/{author}/{slug}` on Save. If that
  fails, the user picks the path ("transcription" / "chat audio").
* **Picker (both apps, same data from Rust):** search by id and name. Filter chips:
  *ZDR only* (default **on**), *Fast providers*, *Free*, *Transcription / Chat audio*
  (STT). Each row shows the name and id, and the price:
  * STT: "$0.04/h audio", or "≈ $0.035/h + output" for chat audio at an assumed
    32 tok/s, marked "≈".
  * Cleanup: "≈ $0.0002 per dictation", computed from the prompt-v2 size (≈ 1 k input
    tokens) + 2×input+32 output.

  Rows also show context length (cleanup), badges `ZDR`, `fast` (a ZDR endpoint on
  Groq/Cerebras/SambaNova), `free (20/min, may log)`, `chat model, may paraphrase`
  (STT path B) and `reasoning` (cleanup, mandatory reasoning → slower). The **local
  median latency** of the last 20 calls per model comes from the usage store (our own
  measurement, more useful than OpenRouter's p50s, which were null for STT). An optional
  per-model detail view fetches `/endpoints` and shows `latency_last_30m` /
  `throughput_last_30m` where they are not null. Sort: recommended first, then price.

### 2.3 Recommended defaults

| Use | Default model | Why | Alternatives in the picker's "Recommended" group |
|---|---|---|---|
| STT | **`openai/whisper-large-v3-turbo`** | ZDR on Groq and DeepInfra; $0.012-0.04/h; Groq ≈ 200× real time (`docs/cloud-stt.md` §2.2); dictionary biasing via `provider.options.groq.prompt` when Groq serves | `elevenlabs/scribe-v2` (ZDR, $0.22/h, top accuracy), `mistralai/voxtral-mini-transcribe-3-0` (ZDR, $0.21/h, EU endpoint), `nvidia/parakeet-tdt-0.6b-v3` (ZDR, $0.09/h, matches our local engine), `deepgram/nova-3` (ZDR) |
| Cleanup | **`meta-llama/llama-3.3-70b-instruct`** with `provider.sort: "latency"` | Non-reasoning, follows few-shot prompts well; ZDR endpoints on Groq (0.59/0.79 $/M) and SambaNova-turbo (0.45/0.90 $/M); ≈ $0.0007 per dictation at Groq prices | `meta-llama/llama-3.1-8b-instruct` (Groq ZDR, 0.05/0.08 $/M, ≈ $0.00006/dictation, weaker), `mistralai/mistral-small-3.2-24b-instruct`, `google/gemini-3.1-flash-lite` (Vertex ZDR, `reasoning.effort: "none"`), `openai/gpt-oss-20b` (Groq ZDR, mandatory reasoning → effort `low`, slower) |

Live latency is **not measured** (no key). The only figure is the formatting finding's
estimate of 0.5-1.5 s total for OpenRouter small models
(`docs/findings/2026-10-24-formatting-gap.md`). The live smoke (§5.3) records real p50/p90
per default and updates this table.

Accuracy is also unmeasured. The STT default's WER on our clips is a DoD item (§5.4): if
whisper-turbo loses to Parakeet-local on the cliptest, Scribe becomes the default.

### 2.4 Cloud STT: one adapter, two paths

`crates/flow-cloud/src/adapters/openrouter.rs` keeps `CloudAdapter` and picks the path per
model from the catalogue (`ModelInfo.stt_path`). A config override is allowed
(`stt_path: "auto" | "transcriptions" | "chat_audio"`).

| | Path A `transcriptions` (default) | Path B `chat_audio` (opt-in per model) |
|---|---|---|
| Request | `POST /audio/transcriptions`, JSON `{model, input_audio{data, format}, language?, temperature: 0, response_format: "json", provider}` (as today) | `POST /chat/completions`, `{model, messages: [system: TRANSCRIBE_PROMPT, user: [{type: input_audio, input_audio{data, format}}, {type: text, text: "Language: <lang or auto>."}]], temperature: 0, max_completion_tokens: ceil(audio_s × 8) + 32, reasoning: {effort: "none"}` if supported else lowest, `stream: false, provider}` |
| Biasing | `provider.options.groq.prompt = "Vocabulary: t1, t2, …"` (whisper models; Groq is the only slug with a documented `prompt`); other slugs: none in v1 | dictionary terms in the text part ("Spell these names exactly: …") |
| Format | FLAC (≈ 50-60 % of WAV). If a 400 says the format is unsupported, retry once with WAV and remember `wav` for that model in the cache | **WAV** (the most widely supported chat-audio format; FLAC not universal per the doc's "vary by provider") |
| Chunking | ≤ 55 s per request (60 s upstream timeout), split at VAD silences (existing code) | same 55 s split, for latency and token limits |
| Cost | `usage.cost` (exact) | `usage.cost` (exact) |
| Guard | provider text → full local pipeline (C5) | the same, **plus** a transcript sanity check before C5. Reject and fall back to the next route if any of these hold: words/s > 6; the output starts with a preamble ("Here is", "Transcript:", quotes); `finish_reason != stop`; or the output is non-empty while VAD speech < 300 ms. Reported as `provider_error` code `chat_audio_unverbatim` |

`TRANSCRIBE_PROMPT` is our own wording: "You are a speech-to-text engine. Output only the
exact words spoken in the audio, in the spoken language, with punctuation. Do not answer,
translate, summarise or add anything. If there is no speech, output nothing."

The picker labels path-B models "chat model, may paraphrase". The first selection shows a
one-line warning. Path B never becomes a default.

**Audio encoding.** We have 16 kHz mono f32 → PCM16. 16 kHz mono 16-bit is 32 KB/s
(1.92 MB/min). For a 10 s dictation:

| Format | Bytes | Base64 body | Upload at 5 Mbit/s up | Encode | Notes |
|---|---|---|---|---|---|
| WAV | 320 KB | 427 KB | ≈ 0.68 s | 0 | universal |
| FLAC (`flacenc` level 5, existing) | ≈ 160-190 KB | ≈ 215-255 KB | ≈ 0.37 s | ≈ ms | lossless; default path A |
| MP3/Opus 24-32 kbps | ≈ 30-40 KB | ≈ 50 KB | ≈ 0.08 s | needs libmp3lame/libopus C builds (rejected in D18 for Android NDK/MSVC) | lossy, can hurt ASR |

FLAC saves ≈ 0.3 s per 10 s on a slow uplink and costs almost nothing, so it stays the
default for path A. Opus stays rejected until a pure-Rust encoder is proven: an
OGG/Opus encoder would cut another ≈ 0.3 s on mobile. That is noted as a follow-up and
not part of D28.

### 2.5 Cloud text cleanup

**Where it plugs in.** flow-core's LLM step is already backend-agnostic: the `Cleaner`
trait (`crates/flow-core/src/cleanup.rs`) and `run_cleanup` (hard timeout on its own
thread, `unwrap_output`, `lists_to_plain`, divergence threshold, `guard::check`).
`engine.rs` decides when it runs: `should_run`, `llm_skip_clean` / `llm_can_help`, prompt
v2 via `build_prompt`, and `llm_timeout_ms`. A cloud backend that implements `Cleaner`
therefore gets prompt v2, the guard, `llm_skip_clean`, the timeout and the fallback text
**unchanged by construction**.

* `crates/flow-cloud/src/cleaner.rs`: `OpenRouterCleaner: flow_core::Cleaner`.
  * `clean_cancellable(prompt, deadline, cancel)` → `POST /chat/completions` with:
    * `messages = prompt.messages()` (system, the few-shot pairs, user);
    * `temperature: 0`, `seed: 0`;
    * `max_completion_tokens = max(16, 2 × tokens(transcript) + 32)` (03 §7.5; the
      16 floor follows the doc note);
    * `reasoning: {effort: "none"}` when the model supports it and reasoning is not
      mandatory, else the lowest supported effort; `include_reasoning: false`;
    * `stream: true` (cancellation stops billing; §1.3);
    * `provider: {zdr: true, data_collection: "deny", sort: "latency"}` by default.
  * The SSE reader skips `:` comment lines, appends `delta.content`, treats an `error`
    event / `finish_reason: "error"` as `CleanError::Backend("provider_error")`, and
    reads `usage.cost` from the final chunk.
  * `cancel` (set by `run_cleanup` on timeout) drops the response body, which aborts the
    stream.
  * `finish_reason: "length"` returns the text as-is. The guard then usually rejects a
    truncated sentence via `Dropped`.
* `prefill(prompt)`: nothing is sent with content. It just warms the pooled TLS
  connection with `GET /api/v1/key`, at most once per 60 s. That also refreshes the
  credit display. `cancel_prefill` is a no-op.
* `info_json()` returns `{engine: "cloud", model, last_cost_usd, last_generation_id,
  last_latency_ms, ttft_ms}`.
* Prompt caching: the v2 system prompt + few-shots are a byte-identical static prefix.
  Providers with implicit caching (`supports_implicit_caching` on endpoints) bill cached
  tokens cheaper. No `session_id` is sent by default: it is a stable routing key, and the
  gain is small for ≈ 1 k-token prompts. It can be revisited if cost data says so.

**Context minimisation (new, required).** `build_prompt` puts in more than the
transcript: up to 300 chars of field text before the cursor, the selection, 150 chars
after, `app_id` and screen words. That is fine on-device. Sent to a third party, it is
the on-screen-text leak that D18 avoided (`send_screen_words` default off). flow-core
change, additive:

```rust
#[derive(Clone, Copy, Default)] pub struct PromptScope { pub surrounding_text: bool, pub screen_words: bool, pub app_id: bool }
impl PromptScope { pub const FULL: Self = Self { surrounding_text: true, screen_words: true, app_id: true }; }
pub trait Cleaner { /* existing */ fn prompt_scope(&self) -> PromptScope { PromptScope::FULL } }
pub fn build_prompt_scoped(text, level, style, ctx, dict, scope: PromptScope) -> CleanPrompt; // build_prompt = FULL
```

`engine.rs` calls `build_prompt_scoped(…, cleaner.prompt_scope())`. `OpenRouterCleaner`
returns the user's choices, by default all `false`: what leaves the device is the
transcript, the app **category** and style, and the dictionary vocabulary. Each extra item
has its own toggle ("Send text around the cursor", "Send on-screen words", "Send app
name"). Password fields never reach the core today, and that is unchanged.

**Routing order for cleanup.** New `CleanupRoute { Local, Cloud }` and
`RouterConfig.cleanup_order: Vec<CleanupRoute>`. Empty = today's behaviour (local LLM if
loaded). The enable flow sets `[cloud, local]`; the user can choose `[local, cloud]`
("cloud only when no local model is loaded").

A generic `flow_core::cleanup::RoutedCleaner { primary, fallback, min_fallback_ms: 800 }`
does the composition. It has no network code and is unit-testable:
* It tries `primary`.
* If `primary` fails **fast** (connect/DNS error, open circuit, 401/402/403, budget,
  `cloud_cleanup_not_configured`) and at least `min_fallback_ms` remain before the
  deadline, it runs `fallback`.
* A timeout or a guard rejection is final, so the deterministic text is used, exactly
  like today. Retrying another LLM after a guard rejection would double cost and latency
  for little gain.
* `prompt_scope()` returns the **intersection** of the two scopes, so one prompt is
  valid for both.

Where the core runs:

| Dictation route | Who cleans | Cloud cleanup applies? |
|---|---|---|
| Client (on-device ASR) | client `FlowCore` | **yes** |
| Cloud STT | `CloudBackend`'s client `FlowCore` (uses `local_core.cleaner()`) | **yes** |
| Server (`flow-server`) | server's own pipeline and LLM | **no** in v1: the server stays no-egress (D17 I6, `IPAddressDeny=any`) and never sees keys (C4). UI says: "Cloud cleanup is used for on-device and cloud dictations. Your server uses its own AI." |

The v2 path for the server route is an additive protocol flag `defer_llm`: the server
returns the post-deterministic, pre-LLM text plus the cleanup inputs, and the client runs
cleaner + post-processing. It is a separate decision, because it moves part of the
pipeline to the client for that route.

**Attaching it.** flow-core gets the additive `FlowCore::set_cleaner(Option<Arc<dyn
Cleaner>>)`, a worker control message that takes effect at the next session. Today the
cleaner is fixed at `load`/`from_engines`. The router gains `with_cleanup_factory(f)` +
`set_cleanup(cfg)`. It builds `RoutedCleaner(OpenRouterCleaner, local LLM)` and installs
it on the local core and on `CloudBackend`'s core. In builds without `cloud` the factory
is absent and nothing changes.

**Consent (separate from STT, D18-style).** Toggle **"Clean up text with OpenRouter"**
(`allow_cloud_cleanup`, default off). The first enable opens a dialog:

> "Send dictated text to OpenRouter? After each dictation, the text (and your dictionary
> words) is sent to OpenRouter (openrouter.ai) and to the model provider it routes to.
> Your voice is not sent unless Cloud speech-to-text is also on. Routed only to
> zero-retention providers by default."

The buttons are Cancel / "Send text to OpenRouter". The dialog records
`cleanup.consent_host` and `consent_at_ms`. STT consent and cleanup consent are
independent records. Turning on one never turns on the other. C1 for cleanup reads:
`caps.cloud && allow_cloud_cleanup && cleanup consent for openrouter.ai && key && budget
&& Cloud in cleanup_order`.

**Timeouts and never-blocking (D24 spirit).**
* The cloud cleanup timeout is its own setting, `cleanup.timeout_ms`, default **2500**,
  range 800-8000. When cloud is primary, the router sets the session's `llm_timeout_ms`
  to it.
* `run_cleanup`'s hard timeout already guarantees that the commit returns the
  deterministic text when the budget is hit (`llm_timeout`), with the stream cancelled.
* Connect timeout is 1.5 s for cleanup, separate from STT's 3 s.
* The cleanup circuit is separate from the STT circuit (auth/quota sticky; 5 s doubling
  to 60 s otherwise). An open circuit means "skip instantly to the fallback/deterministic
  text": no request and no waiting.
* `llm_skip_clean` (default on) means about half of dictations make **no** cloud call at
  all (48/88 on the D27 eval).

**Logging (D21 debug log, additive fields):**
* Per dictation: `llm_engine` (`"local" | "cloud" | null`), `llm_model`, `llm_cost_usd`,
  `llm_ttft_ms`, `llm_generation_id`, `stt_model`, `stt_cost_usd`, `stt_generation_id`.
* `kind: "event"` lines for `cloud_catalog_refresh {source, models, ms}` and
  `cloud_key_info {limit_remaining, usage_monthly}`.
* Never: the key, `Authorization`, request/response bodies outside the existing opt-in
  transcript fields, or provider error text (codes only, as today).

### 2.6 Key, settings schema, budget

**One key, both uses.** Stored as today (Windows Credential Manager
`flow-clone/cloud/openrouter`, bound to host + adapter; Android Keystore AES-GCM
`SecretVault("flow_cloud_openrouter")`). After Save, "Check key" calls `GET
/api/v1/key`. It shows `label`, credit limit / remaining, `usage_monthly` and
`is_free_tier`, and warns if `limit == null`: "This key has no credit limit; OpenRouter
recommends setting one." OAuth PKCE "Connect with OpenRouter" (§1.5) would avoid
copy-pasting the key. It is a follow-up, not v1: it needs a localhost listener on Windows
and a custom-scheme redirect on Android.

`RouterConfig.cloud` schema 2 (keys never in it):

```json
"allow_cloud": false, "allow_cloud_cleanup": false,
"order": [], "cleanup_order": [],
"cloud": {
  "schema": 2,
  "openrouter": {
    "privacy": {"zdr": true, "data_collection": "deny"},
    "stt": {"model": "openai/whisper-large-v3-turbo", "stt_path": "auto", "language": "",
            "send_dictionary": true, "format_override": "", "timeout_base_ms": 6000,
            "consent_host": "", "consent_at_ms": 0},
    "cleanup": {"model": "meta-llama/llama-3.3-70b-instruct", "sort": "latency",
                "timeout_ms": 2500, "send_surrounding_text": false, "send_screen_words": false,
                "send_app_name": false, "consent_host": "", "consent_at_ms": 0}
  },
  "monthly_budget_usd": null
}
```

* `privacy` is shared and shown as one toggle, "Zero-retention providers only", default
  on.
* Turning it off needs a confirm: "Some providers may keep or train on your audio/text."
  Even then `data_collection: "deny"` stays unless a second, explicit toggle is turned
  off.
* `sort` takes `latency | throughput | price | ""` ("" = OpenRouter load balancing).

**Budget: the open "budget for unpriced custom providers" question resolves.** Every
remaining model is an OpenRouter model, and OpenRouter returns `usage.cost` on every
successful STT and chat response (§1.1, §1.3). The new rule:

1. **Pre-flight (start of session / before a cleanup call):**
   * Compute `estimate` from the catalogue.
     * STT: `per_audio_second × max_session_s`, capped by the hard stop. For path B:
       `audio_token_price × 32 × s + completion_price × max_tokens`.
     * Cleanup: `prompt_price × est_tokens(prompt) + completion_price × max_tokens`.
     * Use the **highest ZDR endpoint price** when that is known from `/endpoints`, else
       the model price × 3 as headroom. Endpoint prices can be several × the model price
       (§1.6).
   * Skip the route (`budget_exceeded`) if `month_cost + estimate > budget`.
2. **Ceiling at the source:** every request carries `provider.max_price`, so an
   unexpectedly expensive endpoint is never used. For cleanup that is
   `{prompt, completion}` = 4 × the catalogue price per million. For STT,
   `max_price.audio`/`request` semantics for transcription are unconfirmed, so it is
   omitted there.
3. **Post-hoc:** add `usage.cost` (exact).
4. **No usage in the response** (timeout/abandoned stream, mid-stream error):
   * Record the estimate with `cost_estimated: true`.
   * Queue `GET /api/v1/generation?id=<X-Generation-Id or the stream's id>` once, 30 s
     later and off the dictation path, and replace the estimate with `total_cost`.
   * A failed request with no generation id costs 0. OpenRouter bills failures only as
     prompt processing on a generation that exists.
5. **Unknown-unit models** (§1.1 outliers) and free-text ids missing from the catalogue
   have no estimate. With a budget set, they are allowed only after the first call's real
   cost has been recorded, and then that cost × 2 is used as the estimate.
6. The OpenRouter key's own limit (`limit_remaining`) is shown next to our budget. A 402
   or a 403 with `limit_source` maps to `quota_exceeded`, sticky until the key or the
   month changes, or until `resets_at`.

Budget and usage counters are shared across both uses. They are shown split: "Speech
$x · Cleanup $y · this month".

### 2.7 Error mapping additions (OpenRouter-specific)

| OpenRouter response | `Failure` / `CloudError` | Circuit / retry |
|---|---|---|
| 503 "no provider meets routing requirements" | new **`no_private_route`** | sticky until model/privacy change; hint "No zero-retention provider serves this model. Pick a model marked ZDR." |
| 402 `in_flight_budget_exhausted` | `busy` | one retry if `Retry-After` ≤ 2 s and the deadline allows |
| 402 other, 403 with `limit_source` | `quota_exceeded` | sticky (see §2.6) |
| 403 moderation / guardrail | `provider_error` code `moderation` | none, not sticky |
| 408 | `timeout` | 5 s doubling |
| 502 | `provider_error` | one retry (existing) |
| SSE `finish_reason: "error"` (cleanup) | `CleanError::Backend("provider_error")` → `llm_error: provider_error` | circuit as 5xx |
| 200 empty content (cleanup) | `llm_empty` (existing) | none |

---

## 3. Security

Mostly the existing D18 controls, now with a single host:

* **Key storage:** Windows Credential Manager generic credential (user-scoped), bound to
  host `openrouter.ai` + adapter `openrouter` (`apps/flow-windows/src/credentials.rs`
  binding). Android Keystore AES-GCM `SecretVault`, ciphertext in the backup-excluded dev
  prefs (`apps/flow-android/.../data/DevStore.kt`). Rust holds it only as `SecretString`
  (zeroize on drop, `Debug`/`Display` `***`, not `Serialize`).
* **Host pinning:** with `base_url` a constant, the key-to-host binding cannot be
  redirected by config edits. `validate()` rejects any `base_url` ≠
  `https://openrouter.ai/api/v1` (a future regional-host option would be a closed enum
  `{openrouter.ai, eu.openrouter.ai, us.openrouter.ai}`). The catalogue's `links.next`
  pagination is followed only if same-origin.
* **No redirects:** `reqwest::redirect::Policy::none()` stays (`crates/flow-cloud/src/http.rs`,
  review S3), now also for the catalogue, `/key` and `/generation` clients.
* **TLS:** rustls + webpki-roots, `https` only; unchanged. There is no `http://` exception
  any more, because the keyless-LAN `custom` path is gone.
* **Never logged:** no key, `Authorization` or body in logs, `route_status`, crash text,
  exports or flow-server frames (`flow-contract-tests/tests/no_key_leak.rs` extended to
  the cleaner + catalogue). Provider error codes still pass through `scrub()`.
  `info_json()` carries no key or prompt.
* **Accessibility:** the key field is `SecretField` on Android (`ui/SecretField.kt`:
  a11y text "set (hidden)", no autofill/content capture, scrubbed a11y events) and
  `type=password` on Windows. The `/key` label and limit are shown as plain text; they
  are not secrets. `ui/SecretFieldTest.kt` gains a case for the new page.
* **Data minimisation:** cleanup sends only the transcript + category/style +
  dictionary by default (§2.5). STT sends audio + dictionary terms, and the dictionary
  only to Groq-served whisper. No `user`, no `session_id`, no attribution headers.
* **Build matrix (C2 unchanged + new):** client-only builds still link no flow-cloud. The
  cloud builds must contain no non-OpenRouter provider host strings (§2.1).

---

## 4. Implementation plan (parallel, by file ownership)

Each workstream owns its files. APIs are additive, so they can merge in any order behind
the `cloud` feature. W0 lands first: it is a small API stub plus fixtures that the others
build against.

| # | Owner / branch | Files | Work |
|---|---|---|---|
| W0 | core (`or-core`) | `crates/flow-core/src/cleanup.rs`, `engine.rs`, `lib.rs` | `PromptScope`, `Cleaner::prompt_scope()` (default FULL), `build_prompt_scoped`, `RoutedCleaner` (primary/fallback, fast-failure rule, scope intersection), `FlowCore::set_cleaner`. Tests: scope strips before/selected/after/screen words/app id; RoutedCleaner fallback matrix with fake cleaners (fast fail → fallback; timeout/guard reject → deterministic); `llm_skip_clean` still skips with a RoutedCleaner; FULL scope gives byte-identical prompts (no regression on the D27 88-case eval) |
| W1 | cloud (`or-cloud`) | `crates/flow-cloud/**` | Delete `openai_compat`, `elevenlabs`, `deepgram`, `assemblyai` + fixtures/tests. `catalog.rs` (fetch/parse/cache/baked, search, `stt_path`, ZDR join, unit sanity). `adapters/openrouter.rs` path B (chat audio + sanity check) + `provider.options.groq.prompt` + WAV retry-on-400. `cleaner.rs` `OpenRouterCleaner` (SSE, cancel, usage). `keyinfo.rs` (`/key`), `generation.rs` (deferred cost). Budget estimate helpers replace the `presets.rs` price table. `data/catalog-baked.json` + `tools/gen-openrouter-catalog.py` |
| W2 | router (`or-router`) | `crates/flow-router/src/{cloud,lib,router}.rs` | Schema 2 + `migrate_d28`. `ADAPTERS = ["openrouter"]`, constant base URL. `allow_cloud_cleanup`, `cleanup_order`, cleanup consent/circuit/budget. `with_cleanup_factory`, `set_cleanup`, `cleanup_status()`. Failures `no_private_route`, cleanup `Failure`s. Usage split stt/cleanup. Policy tests: C1 for cleanup (each condition false → no request); STT consent ≠ cleanup consent; server route never uses cloud cleanup; migration fixtures for every old preset |
| W3 | ffi (`or-ffi`) | `crates/flow-ffi/src/{lib,jni_api}.rs`, C header | Additive natives: `nativeCloudCatalog(handle, use, query, filtersJson): String`, `nativeRefreshCloudCatalog(handle, force): String` (blocking; called off Main), `nativeCloudKeyInfo(handle): String`, `nativeSetCleanup(handle, cleanupJson): Boolean`, `nativeCleanupStatus(handle): String`, `nativeMigrateCloud(devJson): String`. `nativeCloudPresets` returns the single OpenRouter entry (kept for ABI) |
| W4 | android (`or-android`) | `apps/flow-android/app/src/main/java/dev/flowclone/{ui/DeveloperActivity.kt, ui/CloudProviderActivity.kt → ui/OpenRouterActivity.kt, ui/ModelPickerActivity.kt (new), data/CloudSettings.kt, data/DevSettings.kt, data/DevStore.kt, FlowApp.kt, ui/SettingsActivity.kt}` | Remove `CloudPresets.ALL`/add-provider picker. One "OpenRouter" page: key (SecretField) + Check key (credits), privacy toggle, **Speech-to-text** card (opt-in + consent, model picker, language, send dictionary, test), **Text cleanup** card (opt-in + its own consent, model picker, sort, timeout, three context toggles, test with a sample sentence), usage split + budget. Settings → AI cleanup row shows "On-device / OpenRouter (<model>) / Off" and the cleanup order. Migration notice. Robolectric tests (consent independence, SecretField, picker search/filter from a fixture catalogue, migration notice) |
| W5 | windows (`or-windows`) | `apps/flow-windows/src/{app,dev_settings,cloud,credentials,prefs}.rs`, `ui/src/settings.ts` (+ committed `ui/dist`) | Same page and semantics as W4. Tauri commands `dev_or_catalog`, `dev_or_refresh_catalog`, `dev_or_key_info`, `dev_or_test_stt`, `dev_or_test_cleanup`, `dev_or_set_cleanup`. Remove `dev_cloud_presets`/`dev_cloud_new_id`, keep `dev_cloud_set_key/clear_key/usage/reset_usage`. Credential migration. AI cleanup panel gains the engine choice. Tests: `cloud_settings`, credential migration, `cloud_integration` vs mock |
| W6 | contract (`or-contract`) | `crates/flow-contract-tests/**`, `tools/check-build-matrix.sh` | Equivalence: cloud cleanup through mock OpenRouter gives the same `CommitResult` as a fake local cleaner with the same output. `no_key_leak` for cleaner/catalogue/key-info. Build-matrix host-string check (§2.1). Toggle matrix: cleanup provider down → deterministic text within timeout + 50 ms |

**Changelog for shells (to append to `docs/cloud-stt.md`):**
* Presets are gone: there is one OpenRouter config (`cloud.openrouter`, schema 2), and
  shells call `nativeMigrateCloud` / Rust `migrate_d28` once.
* The catalogue comes from `nativeCloudCatalog` / `dev_or_catalog` (JSON array of
  `ModelInfo`). Shells hard-code no model lists.
* `allow_cloud_cleanup`, `cleanup_order` and `cloud.openrouter.cleanup` are new. The
  cleanup consent dialog is required before the toggle sticks.
* New `Failure` value `no_private_route`. New D21 fields (§2.5).
* `RouteInfo` gains `llm_engine`, `llm_model` and `llm_cost_usd` (optional, additive).
* Removed: `dev_cloud_presets`, `dev_cloud_new_id`, base-URL editing, the dialect field,
  `price_per_hour_usd`.

### 4.1 API (frozen, agreed between W1 `feat/or-cloud` and W0/W2/W3 `feat/or-route`, 2026-10-24)

flow-cloud (W1) public surface. Router/core types are named as or-route
defines them; flow-cloud keeps its own settings types and converts.

```rust
// crates/flow-cloud/src/api.rs — the only host
pub const BASE_URL: &str = "https://openrouter.ai/api/v1";   // never configurable
pub const HOST: &str = "openrouter.ai";
pub struct Api;  impl Api { fn new(); fn with_connect_timeout(Duration); fn url(&self, path) -> String; fn client(&self) -> &Client }
// test seam, feature `mock-server` only (shells never enable it; build matrix checks):
flow_cloud::testing::override_base_url(base) -> BaseOverride   // process-global, serialised; BaseOverride::set(base)

// settings.rs — field names = §2.6 JSON
pub struct Privacy { zdr: bool, data_collection: String /* "deny" | "allow" */ }   // Default = most private
pub enum SttPath { Transcriptions, ChatAudio }        pub enum SttPathChoice { Auto, Transcriptions, ChatAudio }
pub struct SttSettings { model, stt_path: SttPathChoice, language, send_dictionary, format_override, timeout_base_ms, privacy }
pub struct CleanupSettings { model, sort, timeout_ms, connect_timeout_ms, send_surrounding_text, send_screen_words, send_app_name, allow_fallbacks, privacy }
pub struct CleanupScope { surrounding_text, screen_words, app_id }   // mirrors flow_core::cleanup::PromptScope
pub const DEFAULT_STT_MODEL = "openai/whisper-large-v3-turbo"; DEFAULT_CLEANUP_MODEL = "meta-llama/llama-3.3-70b-instruct";
pub fn valid_model_id(&str) -> bool;

// catalog.rs
pub struct ModelInfo { id, name, context_length, input/output: Modalities, stt_path: Option<SttPath>, price: Price,
    zdr_price_max: Option<Price>, price_unit_ok, zdr, zdr_providers, fast_zdr_providers, reasoning_mandatory,
    reasoning_efforts, supports: ParamSet, max_completion_tokens, free, batch_only, router, allowed_by_account: Option<bool> }
pub struct Price { per_audio_second, per_audio_token, prompt, completion, request: Option<f64> }
pub struct Catalog { fetched_at_ms, source: CatalogSource /*Live|Cache|Baked*/, snapshot_date, account_filtered, models }
impl Catalog { fn baked(); fn get(id); fn stt_models(); fn cleanup_models(); fn search(q, Use, Filters); fn picker(q, Use, Filters) -> Vec<PickerRow>; fn from_api(..); fn to_json/from_json }
pub struct Filters { zdr_only /*default true*/, fast_only, free_only, stt_path: Option<SttPath> }   pub enum Use { Stt, Cleanup }
pub fn refresh(api: &Api, key: Option<&SecretString>) -> Result<Catalog, CloudError>;   // spec's `http: &Client` → `&Api`
pub fn lookup_model(api: &Api, id: &str) -> Result<ModelInfo, CloudError>;               // GET /models/{author}/{slug}/endpoints
pub struct CatalogStore;  impl CatalogStore { fn new(cache_file); fn in_memory(); fn current() -> Catalog /*no network*/;
    fn refresh(key: Option<&SecretString>, force: bool) -> Result<Catalog, CloudError>; fn search_json(use_: &str, query, filters_json) -> String;
    fn status_json() -> String; fn model(id) -> Option<ModelInfo> }
pub fn set_default_store(Option<Arc<CatalogStore>>);  pub fn resolve(id) -> Option<ModelInfo>  // store, else baked

// STT
pub struct CloudAsr;     CloudAsr::new(SttSettings, SecretString) .with_model_info(..) .with_cost_sink(CostSink) .with_cost_lookup_delay(..)
pub struct CloudBackend; CloudBackend::new(SttSettings, SecretString, Option<Arc<dyn Cleaner>>) | ::refusing();  impl flow_router::Backend
pub const ADAPTERS: [&str; 1] = ["openrouter"];  pub const PROVIDER_ID = "openrouter";  pub const TRANSCRIBE_PROMPT
pub fn attach / attach_with / factory / factory_with   // flow_router::CloudFactory
pub fn presets() -> Vec<CloudProviderConfig>          // exactly one OpenRouter entry (ABI of nativeCloudPresets)

// cleanup
pub struct OpenRouterCleaner;  ::new(CleanupSettings, SecretString) .with_model_info .with_cost_sink .with_cost_lookup_delay
impl flow_core::cleanup::Cleaner for OpenRouterCleaner   // + or-route's additive methods once W0 lands (below)
impl OpenRouterCleaner { fn scope() -> CleanupScope; fn last_call() -> Option<CleanupCall>; fn estimate_cost_usd(&CleanPrompt) -> Option<f64>;
    fn key_info() -> Option<KeyInfo>; fn request_body(&CleanPrompt) -> Value }
pub struct CleanupCall { model, cost_usd, cost_estimated, generation_id, ttft_ms, latency_ms, finish_reason, error: Option<&'static str> }
pub fn cleaner_for(CleanupSettings, SecretString) -> Arc<dyn Cleaner>;

// key / cost / budget
pub mod keyinfo { pub struct KeyInfo { label, limit, limit_remaining, limit_reset, usage, usage_daily, usage_weekly, usage_monthly, is_free_tier, free_requests_remaining, checked_at_ms };
                  pub fn fetch(&SecretString) -> Result<KeyInfo, CloudError>; pub fn fetch_with(&Api, &SecretString, Instant) }
pub mod generation { pub fn fetch(&Api, &SecretString, id, Instant) -> Result<GenerationInfo, CloudError>;
                     pub fn schedule(Api, SecretString, CostUse, id, estimated, delay, CostSink); pub type CostSink = Arc<dyn Fn(DeferredCost)+Send+Sync> }
pub mod budget { estimate_stt(&ModelInfo, audio_s); estimate_cleanup(&ModelInfo, prompt_tokens, max_tokens); max_price(&ModelInfo) -> Option<MaxPrice>; cleanup_max_tokens; chat_audio_max_tokens }

// errors
pub enum CloudError { Connect, Auth, Quota, RateLimited{..}, TooLarge, NoPrivateRoute, Moderation, UnsupportedFormat, Unverbatim, BadRequest{..}, Server(u16), Timeout, Malformed }
impl CloudError { fn failure() -> Failure; fn code() -> &'static str /* "no_private_route" etc. */; fn detail(); fn is_fast_unavailable() }
```

Agreed with or-route (it owns these; flow-cloud adopts them when they land):

* flow-router: `Failure::NoPrivateRoute` (`"no_private_route"`); `cloud::ADAPTERS` length 1; schema-2 types
  `OpenRouterConfig { privacy: Privacy, stt: SttConfig, cleanup: CleanupConfig }` with the field names above
  (+ `consent_host`/`consent_at_ms`); `CloudFactory = Box<dyn Fn(&OpenRouterConfig, SecretString) -> Arc<dyn Backend>>`;
  `CleanupFactory = Box<dyn Fn(&OpenRouterConfig, SecretString) -> Arc<dyn Cleaner>>` + `Router::with_cleanup_factory`;
  `Backend::install_cleanup(&self, Option<CleanupPlan>)` with `CleanupPlan = Arc<dyn Fn(Option<Arc<dyn Cleaner>>) -> Option<Arc<dyn Cleaner>>>`
  (default no-op; `CloudBackend` does `core.set_cleaner(plan.and_then(|p| p(core.cleaner())))`).
* flow-core `Cleaner` additions (all defaulted): `prompt_scope()`, `engine() -> &'static str` ("cloud"), `model()`,
  `timeout_ms()`, `estimate_cost_usd(&CleanPrompt)`, `clean_metered(..) -> (Result<String, CleanError>, CleanUsage)`;
  `CleanUsage { engine, model, cost_usd: Option<Usd>, cost_estimated, generation_id, ttft_ms, latency_ms }`;
  `CleanError::Unavailable(code)` for fast failures (`CloudError::is_fast_unavailable`: connect / auth / quota /
  no_private_route / busy), else `Backend(code)`. `Failure::NoPrivateRoute`, `Failure::CloudCleanupNotConfigured`.
* flow-cloud glue: `impl From<&OpenRouterConfig> for SttSettings / CleanupSettings`, `factory_with`,
  `cleanup_factory()`, `attach_with` installs both. `Router::with_cloud_key(|Option<&SecretString>| ..)` lends the key to
  the FFI for `CatalogStore::refresh` / `keyinfo::fetch`; the FFI calls `catalog::set_default_store`.

* flow-router (or-route) also: `Router::{set_cleanup, cleanup_status, cloud_notice, ack_cloud_notice, set_cloud_key_for (legacy)}`,
  `RouterConfig::migrate_d28() -> MigrationReport`, `CloudUsage::migrate_d28`, `RouteStatus.cleanup_circuit`,
  `RouteInfo.{llm_engine, llm_model, llm_cost_usd}`, `CommitResult.llm` (`None` when every cleaner refused up front).
  FFI natives: `apps/flow-android/JNI_CONTRACT.md` "OpenRouter (D28)".

Status: merged on `feat/or-cloud` with `feat/or-route` @ d01c684. Both branches merged into each other (`feat/or-route` @ the merge of 8ebe3f4).

---

## 5. Tests

### 5.1 Mock OpenRouter server fixtures

`crates/flow-cloud/fixtures/openrouter/`, sanitised and dated:

| Fixture | Contents |
|---|---|
| `models_all.json`, `models_transcription.json`, `endpoints_zdr.json` | trimmed from this doc's snapshots |
| `transcriptions_ok.json` (+ `X-Generation-Id`) | transcription response |
| `transcriptions_verbose.json` | verbose transcription response |
| `chat_audio_ok.json`, `chat_audio_preamble.json` | sanity-check reject |
| `chat_stream_ok.sse` | with `: OPENROUTER PROCESSING` comments and a final usage chunk |
| `chat_stream_midstream_error.sse` | mid-stream error |
| `key_ok.json`, `key_no_limit.json` | key info |
| `generation_ok.json` | deferred cost lookup |
| `err_401.json`, `err_402_inflight.json` (+ `Retry-After`), `err_402_credits.json`, `err_403_key_limit.json`, `err_403_moderation.json`, `err_408.json`, `err_429.json`, `err_502.json`, `err_503_routing.json` | errors |

The existing `flow-cloud` `mock-server` feature serves them.

Contract tests assert:
* method/path, `Bearer`, and that no `HTTP-Referer` / `X-OpenRouter-Title` / `user` /
  `session_id` is sent;
* `provider` = `{zdr: true, data_collection: "deny", …}` by default;
* `max_price` is present on cleanup requests;
* the base64 audio round-trips;
* path selection per catalogue entry;
* the WAV retry runs once on a format-400;
* the SSE parser ignores comments;
* cancel aborts the stream within 50 ms;
* the error table §2.7;
* budget pre-flight / post-hoc / deferred generation cost.

### 5.2 Core / router

* W0 and W2 tests above.
* The D27 88-case eval run with `RoutedCleaner(Fixed LLM outputs)` must give unchanged
  results with `PromptScope::FULL`.
* A new eval column "scope minimal" measures the quality cost of not sending the
  surrounding text, using the local 1.5B as a proxy.

### 5.3 Live smoke (optional, costs money, never in CI)

`FLOW_CLOUD_LIVE_OPENROUTER_KEY=… cargo test -p flow-cloud --test live_smoke -- --nocapture`.
Optional `FLOW_CLOUD_LIVE_OPENROUTER_STT_MODEL` / `…_CLEANUP_MODEL`.

* STT: `jfk.wav` through `CloudBackend` for each recommended STT model (path A) and one
  path-B model. It asserts WER ≤ 10 %, and prints latency and `usage.cost`.
* Cleanup: 8 messy dictations through `FlowCore` with the cloud cleaner. It asserts the
  guard passes on ≥ 6/8 and p90 < `timeout_ms`, and prints TTFT, total and cost.
* Key info: `/key` parses.
* It skips with a line when no key is present. No key exists on this box; none was
  looked for.

### 5.4 DoD

1. Workspace + both apps green. Strict build matrix with the host-string check.
2. Migration fixtures for all 7 old presets.
3. Live smoke recorded in this doc (latency/cost/WER per default). Defaults revisited
   with the data.
4. On-device: both opt-ins independently, consent dialogs, credits row, picker search,
   offline baked catalogue, D21 fields present.

---

## 6. Open questions / not confirmed

* Whether `provider.sort`, `max_price` or `preferred_max_latency` are honoured on
  `/audio/transcriptions`. The docs only say that `order`/`only`/`ignore` are **not**
  applied there.
* Whether `data_collection` changes transcription routing at all (the doc lists it with
  no detail).
* That `zdr: true` + a model with no ZDR endpoint returns 503 rather than another code
  (inferred from the error table).
* Chat-audio size/timeout limits and audio tokens/s for non-Gemini models.
* The `mai-transcribe` price units (§1.1). That `-1` means "variable" (routers).
* Whether `X-Title` (old name) still works; only `X-OpenRouter-Title` is documented. Moot,
  since D28 sends neither.
* ETag/`If-None-Match` on `/api/v1/models`.
* Real latency of every default (OpenRouter's endpoint stats were null unauthenticated).
* Whether DeepInfra's whisper accepts a `prompt` option (only `groq.prompt` is
  documented).

---

## 7. Draft D28 row (for `docs/DECISIONS.md`)

| D28 | OpenRouter-only cloud: STT + opt-in text cleanup (user decision 2026-10-24; refines D18, fills D27 item 7; spec `docs/proposals/openrouter-only-cloud.md`) | OpenRouter (`https://openrouter.ai/api/v1`, constant, host-bound key) is the only third party for both cloud uses. The `openai_compat`/`elevenlabs`/`deepgram`/`assemblyai` adapters, presets and `custom` base URLs are **deleted** (build matrix fails on other provider hosts). A one-time `migrate_d28` keeps an OpenRouter entry's key + consent and deletes other providers and their keys, with a notice. Models come from the live catalogue (`/api/v1/models?output_modalities=all` + `/api/v1/endpoints/zdr` (+ `/models/user` with a key), 24 h cache, baked offline fallback, free-text id): STT = 28 transcription models via `/audio/transcriptions` (FLAC, 55 s chunks) or, opt-in per model, 33 audio-input chat models via `input_audio` with a verbatim prompt + sanity check; cleanup = any text model via streamed `/chat/completions`. Default privacy `provider: {zdr: true, data_collection: "deny"}`, no `user`/`session_id`/attribution headers. Defaults: STT `openai/whisper-large-v3-turbo`, cleanup `meta-llama/llama-3.3-70b-instruct` + `sort: latency`. Cloud cleanup = `OpenRouterCleaner: flow_core::Cleaner` behind `RoutedCleaner` (`cleanup_order` `[cloud, local]`), so prompt v2, guard, `llm_skip_clean` and the deterministic fallback are unchanged. It has its own opt-in + consent (text leaves the device; voice only with cloud STT), timeout 2500 ms with stream cancel, its own circuit, a minimal prompt scope by default (no surrounding/screen text or app id), and is not applied on the server route (server stays no-egress). Budget: catalogue estimate pre-flight + `max_price` ceiling + exact `usage.cost` (deferred `/generation` lookup when missing). This resolves the "unpriced custom provider" P3 | user: "the only provider being used is OpenRouter … support as many models as possible from within OpenRouter". One host makes "only OpenRouter" a property of the binary. One catalogue reaches 28 + 33 STT and ~390 cleanup models. ZDR is enforceable per request (269 text / 21 STT models have ZDR endpoints). The existing default `openai/gpt-transcribe` has no ZDR endpoint and likely failed under the default `zdr: true` |
