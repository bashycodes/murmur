# Cloud speech-to-text (opt-in, third party)

Status: design (branch `cloud-design`), decision **D18** in `docs/DECISIONS.md`.
Builds on D17 / `docs/hybrid-architecture.md` (router, `Backend` trait, buffered
fallback) and the in-progress router on `hybrid-server` (`Failure`, `BackendError`,
`Backend::session_failure`, `with_local_capability`, `probe_config`).
Provider facts retrieved 2026-10-21/22 from official docs (URLs in §2).

**Goal.** In Developer settings the user may attach an API key for a third-party STT
service (OpenRouter, ElevenLabs, any OpenAI-compatible `/v1/audio/transcriptions`
service, Deepgram, AssemblyAI) and route dictation to it. This is the only path in the
product where audio reaches a third party, so it is fenced:

| # | Invariant | Enforced by |
|---|---|---|
| C1 | Off by default; never used without `allow_cloud` **and** a recorded per-provider consent **and** a key **and** `cloud` in the route `order` | `flow-router/tests/policy.rs::cloud_*_never_used_*` |
| C2 | Not compiled into client-only builds (Android `local` flavour, Windows `local,app`) | `tools/check-build-matrix.sh` (no `flow-cloud\|reqwest\|hyper\|rustls`), Capabilities `cloud:false` |
| C3 | Keys live only in OS secure storage (Android Keystore AES-GCM, Windows Credential Manager) and in Rust memory; never in `RouterConfig` JSON, prefs export, logs, `route_status`, crash text, or any flow-server frame | `flow-contract-tests/tests/no_key_leak.rs` |
| C4 | The provider call runs on the client; flow-server never sees keys or cloud traffic (D17 I6 + systemd `IPAddressDeny=any` stay true) | §4, `no_key_leak.rs::start_frame_has_no_cloud_fields` |
| C5 | Provider returns raw text only; VAD, hallucination guard, spoken cleanup, dictionary, optional LLM and Wispr post-processing run in local flow-core exactly as for local ASR | `equivalence.rs::equivalence_cloud_mock_http` |
| C6 | Silence is never uploaded (local VAD runs first) | `flow-cloud/tests/session.rs::silence_makes_no_request` |
| C7 | Every dictation that used a provider is visible: pill badge `C`, route row, usage counter | shell UI tests |

Non-goals (v1): cloud LLM cleanup, streaming partials from the provider, OpenRouter
chat-model audio input (`input_audio` parts on `/chat/completions`), server-side proxy.

---

## 1. Summary of choices

| Topic | Choice | Rejected alternative (why) |
|---|---|---|
| Route model | Third engine kind `cloud`; `RouterConfig.order: Vec<Route>` replaces the two-way `prefer` when non-empty | separate "cloud mode" switch (two policies to test) |
| Where the call runs | **Client, directly** to the provider over HTTPS | server proxies to provider (§4) |
| Transport | **Batch upload at commit** of the VAD-trimmed utterance | provider streaming (v2; §5) |
| Encoding | FLAC 16 kHz mono 16-bit in pure Rust (`flacenc`); WAV fallback; raw PCM where the API takes it | Opus (needs libopus C build on Android/MSVC; lossy hurts ASR at low bitrates) |
| Pipeline | `CloudBackend` = `FlowCore::from_engines(CloudAsr, local_cleaner)` — the cloud adapter is just another `AsrEngine` | post-processing the provider's text in the shell (would fork the pipeline) |
| Dictionary | dictionary terms → provider prompt / keyterms; screen words **not** sent by default | always send `initial_prompt` (leaks on-screen text) |
| Crate | `crates/flow-cloud` (feature-gated, `reqwest` blocking + rustls + webpki-roots) | `ureq` (fine too, but hand-rolled multipart; task asked reqwest) |
| Default providers | ElevenLabs Scribe v2 (direct), OpenRouter `/api/v1/audio/transcriptions` → `openai/gpt-transcribe`, Groq `whisper-large-v3-turbo` via the OpenAI-compatible adapter (§2.3) | OpenRouter chat-audio (§2.1); OpenAI `gpt-4o-*`/`whisper-1` (shut down 2027-02-26) |
| Keys | Win: Credential Manager generic credential; Android: Keystore AES-GCM (`SecretVault`, generalised `TokenVault`) | DPAPI blob in `prefs.json` (prefs get copied/exported) |

---

## 2. Provider research (retrieved 2026-10-21/22)

### 2.1 OpenRouter

**Actual STT capability:** a dedicated `POST https://openrouter.ai/api/v1/audio/transcriptions`
endpoint (launched 2026-05-01, updated 2026-07-07), plus the older route of
`input_audio` content parts on `/chat/completions` for audio-capable chat models.

| Item | Fact |
|---|---|
| Auth | `Authorization: Bearer <key>` (same as chat) |
| Request | JSON `{model, input_audio:{data:<raw base64>, format:"wav"\|"flac"\|…}, language?, temperature?, response_format: json\|verbose_json, provider?:{zdr, data_collection, options:{<slug>:{…}}}}`, or OpenAI-style multipart (`file`, `model`, `language`, …) at the same base URL |
| Prompt / biasing | multipart `prompt` is **accepted but ignored**; provider-specific prompt only via JSON `provider.options.<slug>` (doc example: `groq.prompt`) |
| Formats | wav, mp3, flac, m4a, ogg, webm, aac (varies by upstream) |
| Limits | 25 MB inline (larger base64 only for OpenAI/Groq models, or `input_audio.url`); upstream timeout **60 s** per request → chunk long audio |
| Response | `{text, usage:{seconds, total_tokens, input_tokens, output_tokens, cost}}`, `cost` in USD; header `X-Generation-Id` → exact per-request cost for the usage display |
| Streaming | not offered for transcription |
| Models (live `GET /api/v1/models?output_modalities=transcription`, 2026-10-22; price per second of audio as listed) | `openai/gpt-transcribe` 0.000075 ($0.27/h), `openai/gpt-4o-transcribe` (token-priced), `openai/whisper-large-v3-turbo` 0.00000333, `openai/whisper-large-v3`, `openai/whisper-1` 0.0001 ($0.36/h), `elevenlabs/scribe-v2` 0.0000611 ($0.22/h), `deepgram/nova-3`, `assemblyai/universal-3-6-pro`, `mistralai/voxtral-*`, `google/chirp-3`, `google/gemini-3.5-transcribe`, `nvidia/parakeet-tdt-0.6b-v3`, `qwen/qwen3-asr-*`, `x-ai/grok-stt-1.0`, `microsoft/mai-transcribe-*`, … |
| Retention | OpenRouter keeps no prompts unless the account opts into logging; it stores request metadata. `provider.zdr:true` restricts routing to zero-data-retention endpoints; unknown policies are treated as "retains and trains" |
| BYOK | supported (routes with the user's upstream key; OpenRouter platform fee only) |

**Chat-model audio (`input_audio` on `/chat/completions`): rejected.** Formats wav, mp3,
aiff, aac, ogg, flac, m4a, pcm16, pcm24, base64 only. A chat model asked to "transcribe
verbatim" may answer the question it hears, translate, summarise, drop disfluencies,
or invent text on silence/noise, and it is billed per audio token with no per-second
guarantee. flow-core's guardrail exists for the LLM cleanup step, not for ASR. The
dedicated endpoint gives the same models' STT siblings (gpt-transcribe, Gemini
transcribe, Voxtral transcribe) with verbatim semantics, so there is no reason to
accept the risk.

Sources: <https://openrouter.ai/docs/guides/overview/multimodal/stt>,
<https://openrouter.ai/blog/announcements/announcing-audio-apis>,
<https://openrouter.ai/docs/guides/overview/multimodal/audio>,
<https://openrouter.ai/docs/guides/features/zdr>, <https://openrouter.ai/api/v1/models?output_modalities=transcription>.

### 2.2 Direct providers

| Provider | Batch endpoint · auth | Streaming | Formats · max | Biasing field (limits) | Language | Price (2026-10) | Retention / ZDR |
|---|---|---|---|---|---|---|---|
| **ElevenLabs Scribe** | `POST https://api.elevenlabs.io/v1/speech-to-text` multipart, `model_id=scribe_v2`; header `xi-api-key` | `wss://api.elevenlabs.io/v1/speech-to-text/realtime?model_id=scribe_v2_realtime` (pcm_16000 default; commit strategies manual/vad/turn_prediction; ~150 ms) | all major audio/video; raw `pcm_s16le_16`; files to 3-5 GB (docs conflict), 10 h | `keyterms[]`: ≤ 1000 terms, < 50 chars, ≤ 5 words (realtime 50 / 20 chars); +$0.05/h | `language_code`, auto if omitted | $0.22/h batch, $0.39/h realtime | logs/history kept by default; `enable_logging=false` = zero retention, **enterprise only** |
| **OpenAI** | `POST https://api.openai.com/v1/audio/transcriptions` multipart; Bearer. Recommended model `gpt-transcribe`; `whisper-1`, `gpt-4o-transcribe`, `gpt-4o-mini-transcribe` **shut down 2027-02-26** | Realtime WS `session.type="transcription"` (PCM16 24 kHz), `gpt-live-transcribe` $0.017/min; SSE `stream=true` on file requests | mp3, mp4, mpeg, mpga, m4a, wav, webm (**no flac listed**); 25 MB | `gpt-transcribe`: `prompt` + `keywords[]` (one per line, no `<>`/CR/LF); whisper-1: `prompt` ≤ 224 tokens | `gpt-transcribe`: `languages[]`; whisper: `language` | gpt-transcribe $0.0045/min ($0.27/h); 4o-mini $0.003; 4o/whisper $0.006 | no training by default; transcription has no abuse-monitoring retention (Realtime 30 days); ZDR by agreement |
| **Groq** (OpenAI-compatible) | `POST https://api.groq.com/openai/v1/audio/transcriptions`; Bearer; `whisper-large-v3-turbo`, `whisper-large-v3` | none | flac, mp3, mp4, m4a, ogg, wav, webm; 25 MB free / 100 MB dev tier; min billed 10 s | `prompt` ≤ 224 tokens | `language`, auto otherwise | turbo $0.04/h, large-v3 $0.111/h (≈ 200× real time) | ≤ 30 days for troubleshooting/abuse; **ZDR self-serve** (Data Controls) |
| speaches / LocalAI (self-hosted, OpenAI-compatible) | `{base}/v1/audio/transcriptions`; no or optional auth | SSE / realtime variants | as backend | `prompt` | `language` | own hardware | own machine (not third party) |
| Fireworks | current docs show audio only via chat completions; old direct `…/v1/audio/transcriptions` hosts unconfirmed | — | — | — | — | — | — (not a preset; usable via `custom` if it works) |
| **Deepgram** | `POST https://api.deepgram.com/v1/listen?model=nova-3` raw body; `Authorization: Token <key>` | `wss://api.deepgram.com/v1/listen` | 100+ formats incl. flac/wav/linear16; 2 GB, 10 min processing | `keyterm=` repeated (Nova-3/Flux), ≤ 500 tokens total (~100 terms); +$0.08/h | `language` (BCP-47) / `detect_language` | Nova-3 $0.26/h batch, $0.29/h stream | stores data for its model-improvement programme unless `mip_opt_out=true` |
| **AssemblyAI** | `POST /v2/upload` → `POST /v2/transcript` → poll `GET /v2/transcript/{id}` (base `https://api.assemblyai.com`); header `authorization: <key>` | `wss://streaming.assemblyai.com/v3/ws` | most formats; 2.2 GB upload, 10 h | `keyterms_prompt[]` ≤ 1000 phrases, ≤ 6 words | `language_detection` | Universal-3.5 Pro $0.21/h, Universal-2 $0.15/h | audio deleted in 24-48 h, transcripts 30 days; TTL down to 1 h or `DELETE` via API |
| Gemini API | `generateContent` + "transcribe" prompt (inline ≤ 20 MB, Files API 2 GB); `x-goog-api-key` | Live API WS (PCM16 16 kHz) | wav, mp3, aac, ogg, flac, opus, webm…; 9.5 h | free-text prompt only | in prompt | 32 tokens/s audio; e.g. 2.5 Flash $1.00/M audio tokens (≈ $0.12/h) | **free tier trains on data**; paid: no training. Not a preset (chat-style transcription, same risk as §2.1); reachable via OpenRouter `google/gemini-3.5-transcribe` |

Polling (AssemblyAI) adds ≥ 1 round trip; acceptable for v1 but it is the slowest
preset. Groq's 10 s minimum bill makes short dictations cost ≈ $0.0001 each.

**One generic adapter covers** OpenAI, Groq, speaches, LocalAI (and OpenRouter via
multipart, though the dedicated adapter is preferred for `usage.cost`, `zdr` and
`provider.options`). It needs a per-model *dialect*: `whisper` (`language`, `prompt`) vs
`gpt_transcribe` (`languages[]`, `keywords[]`, `prompt`), auto-picked from the model id
(`gpt-transcribe*`, `gpt-live-transcribe*` → `gpt_transcribe`) and overridable.
Own adapters: ElevenLabs, Deepgram, AssemblyAI.

Sources: <https://elevenlabs.io/docs/api-reference/speech-to-text/convert>,
<https://elevenlabs.io/docs/capabilities/speech-to-text>,
<https://elevenlabs.io/docs/api-reference/speech-to-text/v-1-speech-to-text-realtime>,
<https://elevenlabs.io/pricing/api>,
<https://developers.openai.com/api/docs/guides/speech-to-text>,
<https://developers.openai.com/api/docs/guides/realtime-transcription>,
<https://developers.openai.com/api/docs/pricing>, <https://developers.openai.com/api/docs/guides/your-data>,
<https://console.groq.com/docs/speech-to-text>, <https://console.groq.com/docs/your-data>,
<https://fireworks.ai/blog/streaming-audio-launch>, <https://docs.fireworks.ai/guides/video-audio-inputs.md>,
<https://speaches.ai/usage/speech-to-text/>, <https://localai.io/docs/features/audio-to-text/>,
<https://developers.deepgram.com/reference/speech-to-text/listen-pre-recorded>,
<https://developers.deepgram.com/docs/keyterm>,
<https://developers.deepgram.com/docs/the-deepgram-model-improvement-partnership-program>,
<https://deepgram.com/pricing>,
<https://www.assemblyai.com/docs/speech-to-text/pre-recorded-audio>,
<https://www.assemblyai.com/docs/data-retention-and-model-training>, <https://www.assemblyai.com/pricing>,
<https://ai.google.dev/gemini-api/docs/audio>, <https://ai.google.dev/gemini-api/docs/pricing>,
<https://ai.google.dev/gemini-api/terms>.

### 2.3 Recommended presets

| Rank | Preset | Why |
|---|---|---|
| 1 | **ElevenLabs Scribe v2** (direct) | real keyterms (dictionary maps 1:1), $0.22/h, has a realtime path for v2; caveat: zero retention needs enterprise |
| 2 | **OpenRouter** → `openai/gpt-transcribe` | the user's requested key; one key reaches ~25 STT models incl. Scribe, Deepgram, Voxtral, Parakeet; reports exact `usage.cost`; `zdr` routing flag; caveat: no dictionary biasing except via `provider.options` |
| 3 | **Groq** `whisper-large-v3-turbo` (openai_compat) | cheapest ($0.04/h), fastest, self-serve ZDR, `prompt` biasing |
| — | OpenAI `gpt-transcribe`, Deepgram `nova-3`, AssemblyAI, custom OpenAI-compatible | presets available, not defaults |

---

## 3. Routing

### 3.1 Config and route list

```rust
// flow-router additions (all #[serde(default)], additive; §1.2 signatures unchanged)
pub enum EngineKind { Local, Remote, Cloud }                 // "local" | "remote" | "cloud"
pub enum Route { Server, Client, Cloud }                     // "server" | "client" | "cloud"
pub struct RouterConfig { /* existing */ pub allow_cloud: bool /*false*/,
                          pub order: Vec<Route> /*[] = legacy prefer*/, pub cloud: CloudConfig }
pub type CloudFactory = Box<dyn Fn(&CloudProviderConfig, SecretString) -> Arc<dyn Backend> + Send + Sync>;
impl Router {
    pub fn with_cloud(self, f: CloudFactory) -> Self;          // builder, like with_local_capability
    pub fn set_cloud_key(&self, provider_id: &str, key: Option<SecretString>); // never via JSON
    pub fn probe_cloud(&self, p: &CloudProviderConfig, key: Option<SecretString>) -> HealthReport; // "Test provider"
    pub fn cloud_usage(&self) -> CloudUsage;                   // in-memory session counters
}
```

`order` resolution at `start_session`:

1. `order` empty → legacy D17 policy (`prefer` + the other). **Cloud is never implied**: a
   config without `cloud` in `order` cannot reach a provider even with `allow_cloud`.
2. Otherwise walk `order`, skipping routes whose effective flag is off
   (`allow_cloud_eff = caps.cloud && allow_cloud && consent_ok && key_present && budget_ok`).
3. First eligible route is primary; the remaining eligible routes are the fallback chain
   (D17 had at most one fallback; now ≤ 2). Duplicates in `order` → `bad_config`.
4. The enable flow writes `order = ["cloud", <previous primary>, <previous fallback>]`;
   the user can reorder.

| order (all allowed) | Meaning | Extra UI warning |
|---|---|---|
| `cloud, client` | provider first, on-device fallback | — |
| `cloud, server, client` | provider, then own server, then device | — |
| `server, cloud, client` | own server; provider only when the server is down | "When your server is unreachable, audio goes to <provider>" |
| `client, cloud` | device; provider only when no local model is loaded | "When no local model is loaded, audio goes to <provider>" |

### 3.2 Failure handling for the cloud route

Batch upload means the provider is not contacted until commit, so there is no
mid-session cloud failure: `start_session` on the cloud backend only checks
config/key/consent (no network), and the router keeps the D17 PCM buffer for replay.

| When | Trigger | `route.fallback` (new `Failure` values in **bold**) | Circuit |
|---|---|---|---|
| start | no key / no consent / provider not configured | **`cloud_not_configured`** (route skipped; next route) | — |
| start | monthly budget reached | **`budget_exceeded`** | until month or budget changes |
| commit | connect / TLS / DNS error | `connect_failed` | 5 s doubling to 60 s (cloud has its own `Circuit`) |
| commit | HTTP 401/403 | `auth_failed` | sticky until key/config changes |
| commit | HTTP 402, provider "insufficient credits/quota" | **`quota_exceeded`** | sticky until key/config changes |
| commit | HTTP 429 | `busy` (one retry if `Retry-After` ≤ 2 s and deadline allows) | 5 s |
| commit | HTTP 5xx, malformed JSON | **`provider_error`** (one retry on 502/503/504 if deadline allows) | 5 s doubling |
| commit | HTTP 400/413/415/422 | **`provider_error`** (no retry; logged status + provider error code, never body text) | none |
| commit | deadline passed | `timeout` | 5 s doubling |
| commit | 2xx with transcript (even empty) | none — **final**, D17 determinism rule | closes |

Fallback after a cloud commit failure = next route in the chain gets
`start_session(cfg)` + full buffer replay + `set_context(last)` + `commit()`, exactly like
D17's remote→local commit fallback. Fallback **to** cloud from server/local uploads the
same buffer. A route that fails at commit is not retried within the session.

`CloudBackend` detects transport failures without a flow-core change: `CloudAsr` writes
its typed error to a shared `Mutex<Option<BackendError>>`; `CloudBackend::commit` returns
`Err(that)` when the core result is ERROR and the slot is set, else `Ok(result)`.

### 3.3 Deadline

`commit deadline = now + cloud.timeout_base_ms (default 6000) + 0.5 × audio_ms + llm_timeout_ms`,
capped at 90 s. HTTP: connect timeout 3 s; request timeout = deadline − now − llm budget.
OpenRouter documents a 60 s upstream timeout, so uploads > 60 s of audio are chunked (§6.2).

### 3.4 RouteInfo and Capabilities (additive)

* `RouteInfo.engine = "cloud"`; new optional fields `provider: Option<String>` (provider id),
  `model: Option<String>`, `billed_audio_ms: Option<u32>`, `cost_usd: Option<f64>`
  (provider-reported if available, else estimate), `cost_estimated: Option<bool>`.
  `server` stays `None` for cloud; host is shown from config, never the key.
* `Capabilities.cloud: bool`, `Capabilities.cloud_adapters: Vec<String>`.

---

## 4. Where the provider call runs: client (chosen) vs server proxy

| | Client direct (**chosen**) | flow-server proxies |
|---|---|---|
| D17 invariants | unchanged; server stays "no outbound connections" (systemd `IPAddressDeny=any`) | breaks I6 spirit and the systemd allowlist; server needs egress |
| Works without a server | yes (Android `remote`/`hybrid` without server, Windows laptop off-LAN) | no |
| Key location | each device's secure storage | one place (server config) |
| Latency | one hop | extra LAN/Tailscale hop + server queue |
| Protocol change | none | new `start.config.cloud` fields; server must parse keys |
| Equivalence | pipeline runs on the client (C5) | pipeline on server; also fine |

Decision: client direct. Keys are never put in `SessionConfig`, `FieldContext`, or any
`flow-protocol` message; `CloudConfig` lives only in `RouterConfig`, which never crosses
the wire. Revisit only if the user wants one key shared across devices; that would be a
new decision, not a flag.

---

## 5. Batch vs streaming

| | Batch at commit (**v1**) | Provider streaming (v2) |
|---|---|---|
| Fits flow-core | yes: `AsrEngine::transcribe(whole utterance)`; VAD trims first (C6, less cost) | needs a new streaming seam in flow-core or the router |
| Partials | none from the provider; pill shows "Transcribing via <provider>…" | live partials |
| Commit latency | upload + provider decode; ≈ 0.4-1.5 s for a 10 s clip on fast providers (measure in live smoke) | lower (final arrives shortly after end of speech) |
| Cost | billed on trimmed audio | billed on open-socket time for some providers |
| Failure model | one request; trivial fallback replay | mid-session socket failures, like D17 remote |

v1 forces `SessionConfig.partial_interval_ms = None` for cloud sessions, and `CloudAsr`
also returns `AsrError::Transcribe("partials unsupported")` for partial requests
(flow-core additive `AsrRequest.purpose`). Reserved: `CloudAdapter::streaming()`
returning `None` in v1; a v2 adapter for ElevenLabs Scribe realtime / Deepgram /
OpenAI realtime plugs in there.

---

## 6. `crates/flow-cloud`

### 6.1 Layout and dependencies

| File | Content |
|---|---|
| `src/lib.rs` | `CloudAdapter` trait, `CloudAsr` (`impl flow_core::AsrEngine`), `CloudBackend` (`impl flow_router::Backend`), `factory()` |
| `src/secret.rs` | `SecretString` (zeroize on drop, `Debug`/`Display` = `"***"`, no `Serialize`) |
| `src/encode.rs` | f32/i16 → WAV / FLAC (`flacenc`) / raw PCM; chunking at VAD silences |
| `src/adapters/{openai_compat,openrouter,elevenlabs,deepgram,assemblyai}.rs` | one file per wire format |
| `src/presets.rs` | provider presets (§8.2) and static price table |
| `src/usage.rs` | per-provider counters, cost estimate |
| `fixtures/<adapter>/*.json` | sanitised real responses (success, empty, errors) |
| `tests/contract_<adapter>.rs`, `tests/session.rs`, `tests/live_smoke.rs` | §10 |

Deps: `flow-core` (`default-features = false`), `flow-router`, `reqwest` (`default-features
= false`, features `blocking`, `json`, `multipart`, `rustls-tls-webpki-roots`), `flacenc`,
`serde`, `base64`, `zeroize`. No OpenSSL / native-tls (Android NDK + MSVC simplicity);
webpki-roots means system CA changes are not honoured (documented). `reqwest::blocking` is
safe here because `AsrEngine::transcribe` runs on flow-core's plain worker thread.
One `reqwest::blocking::Client` per backend, kept across sessions (pooled TLS connection,
idle 90 s) so the second dictation skips the handshake.

### 6.2 Adapter trait

```rust
pub struct CloudRequest<'a> { pub audio: &'a EncodedAudio, pub language: Option<&'a str>,
    pub terms: &'a [String] /*already capped per adapter*/, pub deadline: Instant }
pub struct CloudTranscript { pub text: String, pub language: Option<String>,
    pub billed_audio_ms: Option<u32>, pub cost_usd: Option<f64>, pub request_id: Option<String> }
pub trait CloudAdapter: Send + Sync {
    fn id(&self) -> &'static str;                            // "openai_compat", "elevenlabs", …
    fn preferred_format(&self) -> AudioFormat;               // Flac | Wav | Pcm16
    fn max_upload_bytes(&self) -> usize;                     // chunk above this
    fn max_chunk_ms(&self) -> u32;                           // e.g. 55_000 for OpenRouter's 60 s
    fn max_terms(&self) -> TermBudget;                       // count / chars / tokens
    fn transcribe(&self, http: &Client, key: &SecretString, req: &CloudRequest) -> Result<CloudTranscript, CloudError>;
    fn probe(&self, http: &Client, key: &SecretString) -> Result<ProbeInfo, CloudError>; // cheap auth check, no audio if possible
}
pub enum CloudError { Connect(String), Auth, Quota, RateLimited{retry_after_ms: Option<u64>},
    TooLarge, BadRequest{status: u16, code: Option<String>}, Server(u16), Timeout, Malformed }
```

`CloudError → Failure` mapping is §3.2. Chunking: if the encoded utterance exceeds
`max_upload_bytes` or `max_chunk_ms`, split at the longest VAD silence ≥ 300 ms nearest
each limit, send chunks sequentially with the same terms, join texts with one space;
`billed_audio_ms` and `cost_usd` sum. 20 min (the session hard stop) of FLAC fits in
≤ 2-3 chunks on every provider.

### 6.3 Encoding

| Format | Size / min (16 kHz mono) | Used by default for |
|---|---|---|
| FLAC (`flacenc`, level 5) | ≈ 0.9-1.2 MB (≈ 50-60 % of WAV, speech-dependent) | openrouter, Groq/speaches/LocalAI (openai_compat), deepgram, assemblyai |
| WAV PCM16 | 1.92 MB | `openai` preset (OpenAI lists no FLAC); fallback when a provider rejects FLAC (`format_override: "wav"`) |
| raw PCM s16le 16 kHz | 1.92 MB, no container | elevenlabs (`file_format=pcm_s16le_16`) when supported, else FLAC |

Encoding runs in `CloudAsr::transcribe` on the VAD-trimmed f32 the core hands it
(`timings.asr_ms` therefore includes encode + upload + provider time; add
`cloud_encode_ms`/`cloud_upload_ms` to the dev page, not to `CommitResult`).

---

## 7. Pipeline and dictionary

`CloudBackend` owns `FlowCore::from_engines(Arc<CloudAsr>, cleaner)` where `cleaner` is
the shell's local cleaner (new flow-core accessor `FlowCore::cleaner() -> Option<Arc<dyn
Cleaner>>`, or `None` in builds without `llm`). So VAD → `CloudAsr` → hallucination guard
→ spoken cleanup → optional local LLM + guard → Wispr post-processing → segments all run
locally, and `CommitResult` has the same shape and semantics as a local result. The
provider gives no `no_speech_prob`; the guard falls back to its VAD evidence (as Parakeet).

**flow-core additive change** (owner S, via CR): `AsrRequest` gains
`dictionary_terms: Vec<String>` (dictionary vocabulary only, deduped, original order),
`screen_terms: Vec<String>` and `purpose: AsrPurpose { Final, Partial }`, all
`Default`. `initial_prompt` is unchanged, so whisper/parakeet are untouched.

Term mapping (`terms` = `dictionary_terms`, plus `screen_terms` only if
`send_screen_words`), capped per adapter after dedupe, dictionary first:

| Adapter | Field | Cap used | Notes |
|---|---|---|---|
| openai_compat, dialect `whisper` (Groq, speaches, LocalAI, whisper-1) | `prompt` = `"Vocabulary: t1, t2, …."` | 224 tokens (≈ 120 words) | Whisper-style prompts bias spelling; no instruction semantics |
| openai_compat, dialect `gpt_transcribe` (OpenAI `gpt-transcribe`) | `keywords[]` one term per entry (CR/LF/`<>` stripped) | 200 terms | `prompt` left empty |
| openrouter | `provider.options.<slug>.prompt` when the model's provider takes one; else none | as upstream | multipart `prompt` is ignored by OpenRouter |
| elevenlabs | `keyterms[]` | 200 terms (API ≤ 1000), < 50 chars, ≤ 5 words; longer terms dropped | +$0.05/h; omitted when the list is empty |
| deepgram (nova-3) | repeated `keyterm=` query params | ≤ 500 tokens (≈ 100 terms) | +$0.08/h |
| assemblyai | `keyterms_prompt[]` | 200 phrases (API ≤ 1000), ≤ 6 words | |

Dictionary **replacements** still run locally after ASR (C5), so a provider without
biasing still gets "wispr → Wispr" style fixes. Snippet triggers are never sent.

---

## 8. Settings

### 8.1 Schema (`RouterConfig.cloud`, JSON; keys are never in it)

```json
"allow_cloud": false,
"order": [],
"cloud": {
  "active": "",                         // provider id; "" = none
  "providers": [{
    "id": "openrouter-1", "preset": "openrouter",       // preset id (§8.2) or "custom"
    "adapter": "openrouter",                            // openai_compat|openrouter|elevenlabs|deepgram|assemblyai
    "dialect": "",                                      // openai_compat only: ""=auto|whisper|gpt_transcribe
    "label": "OpenRouter", "base_url": "https://openrouter.ai/api/v1",
    "model": "openai/gpt-transcribe", "language": "",      // "" = follow SessionConfig.languages
    "send_dictionary": true, "send_screen_words": false,
    "zero_retention": true,                              // adapter-specific flag (§2); hidden when unsupported
    "format_override": "",                               // ""|"wav"|"flac"
    "timeout_base_ms": 6000, "price_per_hour_usd": null, // null = preset price
    "consent_host": "", "consent_at_ms": 0               // set by the consent dialog
  }],
  "monthly_budget_usd": null
}
```

Validation (`CloudConfig::validate`, Rust, mirrored only by fixtures in Kotlin):
`base_url` must be `https://`, except `http://` when `flow_router::net::classify(host)`
is private/tailnet (self-hosted speaches/LocalAI); `consent_ok` iff
`consent_host == host(base_url)` and `consent_at_ms > 0` (changing host or adapter clears
consent); `timeout_base_ms` 1000-60000; ids `[a-z0-9-]{1,32}`, unique; `active` must
exist; `order` has no duplicates; ≤ 8 providers.

Key handling: `Router::set_cloud_key(id, key)` at startup and on Save. Windows:
Credential Manager generic credential `flow-clone/cloud/<id>` (`CRED_PERSIST_LOCAL_MACHINE`,
user-scoped). Android: `SecretVault("flow_cloud_<id>")` AES-GCM, ciphertext in the `dev`
prefs file (backup disabled). UI shows only `••••` + last 4 chars. Deleting a provider
deletes its credential. Keys never appear in exported settings, logs, crash reports or
`route_status`.

### 8.2 Presets

| Preset | adapter | base_url | default model | zero_retention flag |
|---|---|---|---|---|
| `openrouter` | openrouter | `https://openrouter.ai/api/v1` | `openai/gpt-transcribe` | `provider.zdr=true` (default on) |
| `elevenlabs` | elevenlabs | `https://api.elevenlabs.io` | `scribe_v2` | `enable_logging=false` (enterprise only; default off, toggle says so) |
| `groq` | openai_compat | `https://api.groq.com/openai/v1` | `whisper-large-v3-turbo` | — (self-serve ZDR is an account setting; UI links to it) |
| `openai` | openai_compat (dialect `gpt_transcribe`) | `https://api.openai.com/v1` | `gpt-transcribe` | — (ZDR is an account agreement) |
| `deepgram` | deepgram | `https://api.deepgram.com` | `nova-3` | `mip_opt_out=true` |
| `assemblyai` | assemblyai | `https://api.assemblyai.com` | `universal` | delete transcript after fetch |
| `custom` | openai_compat | user | user | — |

### 8.3 Developer settings UI (both apps, same keys)

Section **"Cloud speech-to-text (third party)"**, below the server section. Hidden in
client-only builds (C2).

| Control | Behaviour |
|---|---|
| Banner (always visible in the section) | "Off by default. When on, the audio of your dictations leaves this device and is sent to **<provider label> (<host>)**, along with your dictionary words. It is processed under that provider's terms, not by your server." |
| `allow_cloud` toggle | First enable opens the consent dialog for the active provider: title "Send audio to <provider>?", body = banner + retention line from §2 + "You can turn this off at any time"; buttons **Cancel** / **Send audio to <provider>** (records `consent_host`, `consent_at_ms`). |
| Provider list | add from preset, edit, delete, set active |
| API key | password field, paste, clear; Save stores it in secure storage only |
| Model, Base URL (custom/openai_compat only), Language | text fields with validation |
| Send dictionary words / Send on-screen words | toggles (default on / **off**) |
| Zero retention | toggle where the adapter supports it; text says what it does and whether the plan must allow it |
| Route order | reorderable list Server / On-device / Cloud with the §3.1 warnings |
| **Test provider** | uploads a bundled 3 s public-domain clip (first 3 s of `jfk.wav`, FLAC ≈ 50 KB) with the unsaved form + key, bypassing VAD; shows transcript, latency (encode/upload/total), cost, or the error code with a hint (`auth_failed` → "key rejected", `quota_exceeded` → "out of credits", `connect_failed` → "no internet / base URL wrong") |
| Usage | per provider: this month's dictations, billed minutes, cost (reported or "≈ estimated"); Reset counters; optional monthly budget |
| Active engine row | D17 row gains `cloud · <provider> · <model> · $0.0012` |

Normal UI: pill/bubble badge `C` (always, not only with `show_engine_badge`) while the
active route is cloud; tooltip "Sent to <provider>". Errors: "Cloud STT failed
(<reason>); used <fallback>".

Persistence: Windows `prefs.json` `developer.cloud` (no keys), Tauri commands
`dev_cloud_test`, `dev_cloud_set_key`, `dev_cloud_clear_key`, `dev_cloud_usage`.
Android `DevSettings.cloud` + `ui/CloudProviderActivity.kt`, natives in §11.

Usage counters persist locally per month (`cloud_usage.json` / `cloud_usage` prefs):
`{provider_id, month, requests, billed_audio_ms, cost_usd, cost_estimated}`; never sent
anywhere.

---

## 9. Build matrix and permissions

| Build | flow-cloud linked | Network permission | `Capabilities.cloud` |
|---|---|---|---|
| Android `local` | **no** | none (unchanged gate) | false |
| Android `hybrid` | yes (`flow-ffi` feature `cloud`) | `INTERNET` only (unchanged gate) | true |
| Android `remote` | yes | `INTERNET` only | true |
| Windows `local,app` (client-only) | **no** | — | false |
| Windows `remote,app` | no unless `cloud` added | — | per feature |
| Windows default `local,remote,cloud,app` | yes | — | true |

`tools/check-build-matrix.sh` additions: `cargo tree -p flow-ffi -e normal --features
jni,whisper` and `apps/flow-windows --features local,app` must not contain
`flow-cloud|reqwest|hyper|rustls|webpki`; `--features jni,remote,cloud` must still not
contain `whisper-rs-sys|sherpa-onnx|llama-cpp`. No new Android permission; HTTPS from
native code is not subject to Network Security Config, so the `https://` rule in §8.1 is
the gate.

Logging: `flow-cloud` logs `provider_id`, adapter, model, HTTP status, provider error
code, bytes, audio ms, latency, cost. Never the key, `Authorization` header, request
body, response body or transcript (`reqwest` built without its own logging feature;
no `Debug` of requests).

---

## 10. Tests

| File | Owner | What |
|---|---|---|
| `crates/flow-cloud/tests/contract_<adapter>.rs` (one per adapter) | S | `httpmock` server; asserts method, path, auth header name/scheme, content-type, multipart/JSON/query fields (`model`, `language`, terms mapping and caps, format, zero-retention flag), and parses fixture responses; error mapping 401/402/403/413/422/429(+Retry-After)/500/502/timeout/malformed → `CloudError`; one retry rules; chunking (> `max_upload_bytes`) sends N requests and joins text |
| `crates/flow-cloud/tests/session.rs` | S | `CloudBackend` + mock HTTP: silence → EMPTY `no_speech`, **0 requests** (C6); partial requests refused; deadline honoured; transport error → `Err(BackendError)` |
| `crates/flow-cloud/tests/encode.rs` | S | FLAC round-trip bit-exact (decode with `claxon` dev-dep); WAV header; size ratio sanity |
| `crates/flow-router/tests/policy.rs` (+cloud) | S | order walking; cloud never used when any of C1's four conditions is false; `order` empty ignores cloud; budget; per-route circuits; cloud→client replay; server→cloud only when ordered |
| `crates/flow-contract-tests/tests/equivalence.rs` | S | `equivalence_cloud_mock_http`: every session fixture with ASR text T via mock HTTP == `MockAsr(T)` local, after `normalize` |
| `crates/flow-contract-tests/tests/toggle_matrix.rs` | S | `matrix_cloud_*`: 401 → `auth_failed` sticky; 500 → local with 100 % samples; server down + `order=[server,cloud,client]` → cloud; airplane (connect refused) → local |
| `crates/flow-contract-tests/tests/no_key_leak.rs` | S | sentinel key `sk-LEAKCHECK-…`: absent from every serialised `RouterConfig`, `RouteStatus`, `HealthReport`, `Capabilities`, `CommitResult`, flow-protocol frames captured from the in-process server, captured `tracing` output, `Debug` of all config types, and the error strings of every failure path |
| `crates/flow-router/fixtures/router_config_cloud.json`, `route_status_cloud.json`, `capabilities_cloud.json` | S | shared fixtures |
| `crates/flow-cloud/tests/live_smoke.rs` | S | runs only when `FLOW_CLOUD_LIVE_<PRESET>_KEY` is set (`OPENROUTER`, `ELEVENLABS`, `GROQ`, `OPENAI`, `DEEPGRAM`, `ASSEMBLYAI`; optional `…_MODEL`); otherwise returns early with a skip line. Transcribes `jfk.wav` through the full `CloudBackend` with the dictionary `["Meadowcroft"]`; asserts normalised WER ≤ 10 % vs the reference, prints latency and cost. Never in default CI |
| `apps/flow-windows/tests/cloud_settings.rs` | W | prefs round-trip without keys; `CredentialStore` trait with in-memory fake; UI JSON == `router_config_cloud.json` |
| `apps/flow-android/.../CloudSettingsTest.kt`, `SecretVaultTest.kt` | A | `DevSettings.cloud` → JSON equals fixture; consent cleared on host change; vault with JVM fake keystore |

---

## 11. Work split (additive on top of the hybrid branches)

| Implementer | Branch | Owns | Deliverables |
|---|---|---|---|
| **S** (server/core) | `cloud-core` from `hybrid-server` | `crates/flow-cloud/**`, router additions, flow-core CR (`AsrRequest` fields, `FlowCore::cleaner()`), `RouteInfo`/`Capabilities` fields, fixtures, build-matrix script, contract/equivalence/matrix/leak tests | 1. flow-core CR + router `Route`/`order`/`with_cloud`/`set_cloud_key`/`probe_cloud`/`cloud_usage` with fake-backend policy tests and fixtures (unblocks W/A). 2. flow-cloud with openai_compat + openrouter + elevenlabs adapters, encode, contract tests. 3. deepgram + assemblyai adapters. 4. equivalence/matrix/leak tests, live smoke |
| **W** (Windows) | `cloud-windows` from `hybrid-windows` | `apps/flow-windows/**` | Cargo feature `cloud`; `CredentialStore` (Credential Manager via `windows` crate); prefs `developer.cloud`; Tauri commands (§8.3); Settings → Developer cloud section, consent dialog, route-order list, Test provider, usage; overlay `C` badge |
| **A** (Android + FFI) | `cloud-android` from `hybrid-android` | `apps/flow-android/**`, `crates/flow-ffi/**` | flow-ffi feature `cloud` (in `hybrid`/`remote` flavour builds only); natives `nativeSetCloudKey(h, id, key?)`, `nativeProbeCloud(providerJson, key?): String`, `nativeCloudUsage(h): String`; C: `flow_set_cloud_key`, `flow_probe_cloud`, `flow_cloud_usage`; `SecretVault` (generalised `TokenVault`); `DevSettings.cloud`; `CloudProviderActivity`; bubble `C` badge; `build-native.sh --flavor` passes `cloud` for hybrid/remote |

Order: S step 1 lands first (W/A code against it with stubs, as in D17 §6.3); S steps 2-4,
W and A then proceed in parallel. Each merge green under the D17 §6.3 commands plus
`cargo test -p flow-cloud`.

**Definition of done** (new `PROJECT_CONTEXT.md` "What works" rows):
1. Contract tests for all five adapters + `no_key_leak.rs` + cloud policy/matrix/equivalence green.
2. Live smoke green for at least OpenRouter, ElevenLabs and Groq with real keys (WER ≤ 10 % on jfk.wav), latency and cost recorded in this doc.
3. Windows default build: enable cloud with consent, dictate → route `cloud`, badge `C`; revoke key → `auth_failed` → local fallback with no lost words.
4. Android `local` APK: no flow-cloud symbols, no network permission (gates green); `hybrid` APK: same flow as 3.

---

## Changelog for shells

Implementation notes for W and A against the core/router side (branch `cloud-core`,
from `integration` after the three hybrid merges). Every item is additive: shell code
written against the merged D17 router compiles and behaves unchanged; nothing below is
reachable until a shell enables cloud.

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
