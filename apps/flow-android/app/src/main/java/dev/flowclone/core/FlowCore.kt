package dev.flowclone.core

/**
 * Kotlin mirror of the shared Rust core API (windows-research 10-clone-spec §9 v2;
 * crates/flow-core). One loaded [FlowCore] = one §9 `Handle`; one active session at a time.
 *
 * Implementations: [NativeFlowCore] (JNI → libflow_ffi.so, see JNI_CONTRACT.md) and
 * [FakeFlowCore] (tests / "mock engine" debug setting).
 */
interface FlowCore {
    /** `start_session(h, SessionConfig)`. @throws CoreException */
    fun startSession(cfg: SessionConfig)

    /**
     * `push_audio(h, pcm16_16k_mono)`. Non-blocking (called from the AudioRecord thread).
     * Returns false if no session is active or samples were dropped.
     */
    fun pushAudio(pcm: ShortArray, length: Int = pcm.size): Boolean

    /** `set_context(h, FieldContext)`; may arrive after audio started. */
    fun setContext(ctx: FieldContext)

    /** `partial(h)` live preview, or null. */
    fun partial(): String?

    /**
     * `start_session_info` (2026-10-22): [startSession] that also says which route the session
     * starts on. Blocking on the server route: never call it on Main (review C2).
     * @throws CoreException
     */
    fun startSessionInfo(cfg: SessionConfig): StartInfo { startSession(cfg); return StartInfo() }

    /**
     * `poll(h)` (2026-10-22): one non-blocking read per preview tick: partial, hands-free
     * endpoint (D19) and the live route. Engines without it report just the partial.
     */
    fun poll(): CorePoll = CorePoll(partial = partial())

    /** `commit(h)`: blocks until ASR (+cleanup) finishes. Never throws; errors are [CommitStatus.ERROR]. */
    fun commit(): CommitResult

    /** `cancel(h)`. */
    fun cancel()

    /** `learn_edit(h, pasted, edited)`. */
    fun learnEdit(pasted: String, edited: String): List<LearnCandidate>

    /** `unload(h)`: frees models. The instance is unusable afterwards. */
    fun unload()

    /**
     * D24 never-fail fallback (router handles): load the on-device engine on demand from
     * [configs] (tried in order) when a session needs the client route and none is warm.
     * False when this engine can't (bare core, old library).
     */
    fun setLocalLoader(configs: List<ModelConfig>): Boolean = false

    /** D24: open the server connection ahead of a dictation. **Blocking**: never on Main. */
    fun prewarm(): Boolean = false

    /** D24: routing details of the last committed session (router handles), else null. */
    fun lastRouteDiag(): RouteDiag? = null

    /**
     * Command Mode (2026-10-28, `nativeCommand`): rewrite [selected] as [instruction] says, or
     * (null / blank [selected]) write what it describes. Uses the AI cleanup route (on-device
     * LLM, or OpenRouter only when "send text around the cursor" is on). **Blocking** (up to
     * ~25 s): never on Main. Engines without it answer [CommandOutcome.noLlm].
     */
    fun command(instruction: String, selected: String?): CommandOutcome = CommandOutcome.noLlm
}

/** `load_models(ModelConfig) -> Handle`. */
fun interface FlowCoreLoader {
    /** @throws CoreException when the model (or native library) can't be loaded. */
    fun loadModels(cfg: ModelConfig): FlowCore
}

/**
 * [kind] is the stable `CoreError::kind()` (`model_not_found`, `llm_lib_not_found`, `load`,
 * `bad_config`, …) or a router error kind (`no_engine_enabled`, `local_not_ready`,
 * `server_not_configured`, `server_unavailable`); null when unknown.
 */
class CoreException(message: String, val kind: String? = null) : Exception(message) {
    companion object {
        /**
         * Typed load errors (core-p1 CR-1). The JNI message keeps a stable prefix per kind
         * ("model not found: <path>", "llm plugin not found (…)"), so the kind can be
         * recovered even from a library without `nativeLastErrorKind`.
         */
        fun kindOf(message: String?): String? = when {
            message == null -> null
            message.startsWith("model not found:") -> "model_not_found"
            message.startsWith("llm plugin not found") -> "llm_lib_not_found"
            message.startsWith("bad model config") || message.startsWith("bad config") -> "bad_config"
            message.startsWith("model load failed") -> "load"
            else -> RouterErrors.KINDS.firstOrNull { message.startsWith(it) }
        }

        fun of(message: String?, kind: String? = null, fallback: String): CoreException {
            val m = message?.takeIf { it.isNotBlank() } ?: fallback
            return CoreException(m, kind?.takeIf { it.isNotBlank() && it != "error" } ?: kindOf(m))
        }
    }
}

/** flow_router `RouterError` kinds (docs/hybrid-architecture.md §1.2), surfaced as last error. */
object RouterErrors {
    const val NO_ENGINE_ENABLED = "no_engine_enabled"
    const val LOCAL_NOT_READY = "local_not_ready"
    const val SERVER_NOT_CONFIGURED = "server_not_configured"
    const val SERVER_UNAVAILABLE = "server_unavailable"
    const val BAD_CONFIG = "bad_config"
    val KINDS = listOf(NO_ENGINE_ENABLED, LOCAL_NOT_READY, SERVER_NOT_CONFIGURED, SERVER_UNAVAILABLE, BAD_CONFIG)

    /** User-facing wording for review S6's "plaintext refused" (ws:// / http:// to a non-private address). */
    const val NEEDS_TLS = "Server needs TLS — turn on Use TLS for this server in Developer settings"
    const val CLOUD_NEEDS_TLS = "Provider needs TLS — use an https:// base URL"

    /** flow-remote / flow-cloud refused cleartext to an address that is not private or a VPN address. */
    fun plaintextRefused(message: String?) = message?.contains("plaintext refused", ignoreCase = true) == true

    /** D24: the router names the phone-model failure after the server's (`…; phone model failed to load: why`). */
    private const val PHONE_MODEL_FAILED = "phone model failed to load:"

    private fun serverUnavailable(message: String): String {
        val detail = message.substringAfter(':', message).trim()
        val i = detail.indexOf(PHONE_MODEL_FAILED)
        if (i < 0) return "Server unreachable ($detail)"
        val server = detail.substring(0, i).trim().trimEnd(';').trim()
        val phone = detail.substring(i + PHONE_MODEL_FAILED.length).trim()
        return "Server unreachable ($server) and the phone model failed to load ($phone)"
    }

    /** User-facing text for the bubble / main screen (§5 "Errors in normal UI"). */
    fun userMessage(kind: String?, message: String): String = if (plaintextRefused(message)) NEEDS_TLS else when (kind) {
        NO_ENGINE_ENABLED -> "Dictation disabled — enable server or client in Developer settings"
        SERVER_UNAVAILABLE -> serverUnavailable(message)
        SERVER_NOT_CONFIGURED -> "No server configured — set the host in Developer settings"
        LOCAL_NOT_READY -> "On-device speech model is not ready"
        "model_not_found" -> "Speech model file is missing (${message.substringAfter(':').trim()}). Re-import it."
        else -> message
    }
}

/**
 * `ModelConfig.asr_engine`: whisper.cpp (any language) or Parakeet-TDT (English, D12).
 * [SHERPA_ONLINE] (zipformer, D33) is only used for a per-language model (Hebrew), never as the
 * user's default engine.
 */
enum class AsrEngine(val wire: String) { WHISPER("whisper"), PARAKEET("parakeet"), SHERPA_ONLINE("sherpa_online") }

/** crates/flow-core `LanguageModel` (D33): one entry of [ModelConfig.languages]. */
data class LanguageModel(
    val asrEngine: AsrEngine,
    /** The sherpa-onnx model directory (or whisper file). */
    val asrPath: String,
    val parakeetDecoding: ParakeetDecoding = ParakeetDecoding.AUTO,
)

/** crates/flow-core `ModelConfig`. [mockText] selects the core's `Mock` ASR backend. */
data class ModelConfig(
    /** whisper: the ggml-*.bin file; parakeet: the sherpa-onnx model directory. */
    val asrPath: String,
    val llmPath: String? = null,
    val threads: Int = 4,
    val mockText: String? = null,
    /** flow-llm plugin path; null = the core's lookup (on Android: the packaged libflow_llm.so). */
    val llmLib: String? = null,
    val asrEngine: AsrEngine = AsrEngine.WHISPER,
    /** D22: Parakeet search mode; [ParakeetDecoding.AUTO] is omitted from the JSON. Whisper ignores it. */
    val parakeetDecoding: ParakeetDecoding = ParakeetDecoding.AUTO,
    /** D33 per-language models keyed by ISO 639-1 code (`he`); omitted from the JSON when empty. */
    val languages: Map<String, LanguageModel> = emptyMap(),
    /**
     * H5, shell-only (flow-ffi): load only the model this language routes to
     * (`FlowCore::load_models_for`), so the phone keeps one ASR model in RAM. Null = load all.
     */
    val loadLanguage: String? = null,
) {
    /** The ASR model this config actually loads: the [loadLanguage] entry, else the default. */
    val routed: LanguageModel
        get() = loadLanguage?.let { languages[it] } ?: LanguageModel(asrEngine, asrPath, parakeetDecoding)
}

/**
 * crates/flow-core `ParakeetDecoding` (2026-10-23). AUTO loads a second recognizer (≈ +0.6 GB RSS)
 * so hotword-free decodes run greedy and an empty beam result is retried greedy; GREEDY keeps one
 * recognizer without hotword biasing; BEAM is the pre-D22 behaviour.
 */
enum class ParakeetDecoding(val wire: String) {
    AUTO("auto"), GREEDY("greedy"), BEAM("beam");

    companion object {
        fun fromWire(w: String?): ParakeetDecoding? = entries.firstOrNull { it.wire == w }
    }
}

enum class CleanupLevel(val wire: String) { NONE("none"), LIGHT("light"), MEDIUM("medium"), HIGH("high") }

enum class Style(val wire: String) {
    FORMAL("formal"), CASUAL("casual"), VERY_CASUAL("very_casual"), EXCITED("excited");

    companion object {
        fun fromWire(w: String?): Style? = entries.firstOrNull { it.wire == w }
    }
}

/**
 * Per-bucket style (flow-core `Styles`). D27 layer 1: chat buckets default to CASUAL (Wispr
 * Android's own personal default; no final period in chats), email and everything else FORMAL.
 */
data class Styles(
    val personal: Style = Style.CASUAL,
    val work: Style = Style.CASUAL,
    val email: Style = Style.FORMAL,
    val other: Style = Style.FORMAL,
) {
    /** flow-core `Styles::for_category` on the [FieldContext.appCategory] wire name. */
    fun forCategory(category: String): Style = when (category) {
        "personal_chat" -> personal
        "work_chat" -> work
        "email" -> email
        else -> other
    }
}

/** Shared dictionary/snippet export row `{word, replacement, isSnippet}` (§9). */
data class DictEntry(val word: String, val replacement: String? = null, val isSnippet: Boolean = false)

/**
 * flow_router `SessionRoute`: the route a live session is on (`nativeStartSessionInfo` /
 * `nativePoll`). [provider]/[model] only for `cloud`, [server] only for `remote`; [fallback] =
 * why the first route is not the one in use.
 */
data class SessionRoute(
    /** "local" | "remote" | "cloud" */
    val engine: String,
    val provider: String? = null,
    val model: String? = null,
    val server: String? = null,
    val fallback: String? = null,
    /** Shell-side (not in the route JSON): StartInfo / RouterPoll `loading` (D24 on-demand phone model). */
    val loading: Boolean = false,
)

/** `StartInfo` JSON: `queued` = the server accepted but has no free worker yet. */
data class StartInfo(
    val queued: Boolean = false,
    val route: SessionRoute? = null,
    /** D24: the server is unreachable and the phone model is loading on demand ("Loading phone model…"). */
    val loading: Boolean = false,
)

/** `RouterPoll` JSON (`SessionPoll` + live `route`). */
data class CorePoll(
    val partial: String? = null,
    /** D19, latched: the hands-free endpoint fired; finish the dictation. Always false without `endpoint_silence_ms`. */
    val endpointReached: Boolean = false,
    val speechMs: Long = 0,
    val trailingSilenceMs: Long = 0,
    /** Absent with no session, or mid-recording when every route failed (keep the last label). */
    val route: SessionRoute? = null,
    /** D24: the phone model is being loaded for this session (audio is buffered). */
    val loading: Boolean = false,
)

/** flow_router `RouteDiag` (D24): how the last committed session was routed, for the D21 debug log. */
data class RouteDiag(
    /** Time the start spent on the server (connect + hello + start); null = not tried. */
    val connectMs: Long? = null,
    /** The phone model was loaded on demand for this session. */
    val localCold: Boolean = false,
    val localLoadMs: Long? = null,
    val localLoadError: String? = null,
)

data class SessionConfig(
    val languages: List<String> = emptyList(),
    val dictionary: List<DictEntry> = emptyList(),
    val styles: Styles = Styles(),
    val cleanupLevel: CleanupLevel = CleanupLevel.NONE,
    val rawMode: Boolean = false,
    /** Hard core-side bound; the shell owns the real cap (5 min). */
    val maxSamples: Long = 300L * 16_000,
    val partialIntervalMs: Int? = null,
    val llmTimeoutMs: Long = 1500,
    // core-p1 session flags (all default on in the core; the shell always sends them).
    val vad: Boolean = true,
    val hallucinationGuard: Boolean = true,
    /** Deterministic spoken cleanup (punctuation words, fillers, backtracking, ITN). User pref. */
    val spokenCleanup: Boolean = true,
    val llmPrefill: Boolean = true,
    /**
     * D19 hands-free endpointing: trailing silence (ms, 300-10000) after which `poll()` reports
     * `endpoint_reached`. Hands-free sessions only; null (omitted from the JSON) for push-to-talk.
     */
    val endpointSilenceMs: Int? = null,
    /**
     * D22 "Decode while speaking": decode each finished phrase while the user keeps talking,
     * cutting in pauses of at least this long (shell sends 800). Null (omitted) = off. The core
     * turns it off by itself for engines that must not see audio before commit (cloud, D18).
     */
    val segmentPauseMs: Int? = null,
)

/** crates/flow-core `FieldContext`; [before]/[after] ≤ 500 chars (shell truncates). */
data class FieldContext(
    val appId: String = "",
    val urlHost: String = "",
    /** email | work_chat | personal_chat | ai_prompt | terminal | document | other */
    val appCategory: String = "other",
    val before: String = "",
    val selected: String = "",
    val after: String = "",
    val screenWords: List<String> = emptyList(),
)

enum class CommitStatus { FORMATTED, RAW_TRANSCRIPT, EMPTY, ERROR }

data class Timings(
    val audioMs: Long = 0,
    val asrMs: Long = 0,
    val llmMs: Long = 0,
    val postprocessMs: Long = 0,
    val totalMs: Long = 0,
    val droppedSamples: Long = 0,
    /** D22: commit waited this long for a decode already in flight (preview / segment); part of [totalMs]. */
    val waitMs: Long = 0,
    /** D22: pieces that went to ASR (closed segments + tail); 0 = segmentation off. */
    val segments: Long = 0,
    /** D22: ASR time spent while recording on pieces whose text was used (not part of [asrMs]). */
    val preAsrMs: Long = 0,
    /** 2026-10-24: core VAD speech in the dictation (0 = none found); null = VAD off / older core. */
    val speechMs: Long? = null,
    /** 2026-10-24: peak sample level in dBFS (-120 = digital silence); null = older core / no audio. */
    val peakDbfs: Int? = null,
)

/** A key the shell sends between text runs (crates/flow-core `paste::PasteKey`). */
enum class PasteKey(val wire: String) {
    /** Spoken "press enter"; always last. */
    ENTER("Enter"),
    TAB("Tab"),
    /** Paste boundary: finish inserting, pause briefly, continue. No key is sent. */
    BREAK("Break"),
}

/** CommitResult.segments entry: `{"text": "…"}` or `{"key": "Enter"|"Tab"|"Break"}` (CR-2). */
sealed interface PasteSegment {
    data class Text(val text: String) : PasteSegment
    data class Key(val key: PasteKey) : PasteSegment
}

/**
 * CommitResult.route (D17): which engine produced the result. Set by flow_router only;
 * null when the result came straight from a FlowCore handle (or an older native lib).
 */
data class RouteInfo(
    /** "local" | "remote" | "cloud" (D18) */
    val engine: String,
    /**
     * connect_failed, auth_failed, proto_mismatch, busy, timeout, disconnected, circuit_open, server_not_configured,
     * local_not_ready; D18: cloud_not_configured, budget_exceeded, quota_exceeded, provider_error
     */
    val fallback: String? = null,
    /** "host:port" */
    val server: String? = null,
    val rttMs: Long? = null,
    val queueMs: Long? = null,
    val serverCore: String? = null,
    /** D18, cloud results only: provider id, model, billed audio, cost (reported, or estimated when [costEstimated]). */
    val provider: String? = null,
    val model: String? = null,
    val billedAudioMs: Long? = null,
    val costUsd: Double? = null,
    val costEstimated: Boolean? = null,
    /**
     * D22, router only: wall time of `Router::commit` on the phone = release → result including
     * the network for server results (a server's own `total_ms` excludes it). The latency to show.
     */
    val clientMs: Long? = null,
    /** D28: who ran the AI cleanup: "local" | "cloud" (OpenRouter) | "server"; null = no LLM step. */
    val llmEngine: String? = null,
    val llmModel: String? = null,
    /** D28: what the cloud cleanup cost (exact `usage.cost`, else an estimate). */
    val llmCostUsd: Double? = null,
    /** D30, only when AI cleanup (cloud) was tried: what produced the text, "cloud" | "local" | "rules". */
    val cleanupRoute: String? = null,
    /** D30: why AI cleanup's answer was not used (llm_timeout, llm_unsupported, llm_error: <code>, …). */
    val cloudFallback: String? = null,
    /**
     * 2026-10-27, debug log only: AI cleanup's answer that was not used, and the guard's detail
     * (unsupported sentence / mark kind, dropped clause, timeout, error). Dictated text: D21 rules.
     */
    val rejectedText: String? = null,
    val guardDetail: String? = null,
)

/** D28 `CommitResult.llm` (`flow_core::CleanUsage`): the LLM step's usage; null when it did not run. */
data class CleanUsage(
    val engine: String = "local",
    val model: String? = null,
    val costUsd: Double? = null,
    val costEstimated: Boolean = false,
    val generationId: String? = null,
    val ttftMs: Long? = null,
    val latencyMs: Long? = null,
)

data class CommitResult(
    val status: CommitStatus,
    val asrText: String = "",
    val finalText: String = "",
    val language: String? = null,
    val timings: Timings = Timings(),
    val pressEnter: Boolean = false,
    val fallbackReason: String? = null,
    val truncated: Boolean = false,
    val error: String? = null,
    /** Paste plan: [finalText] split at {{BREAK}}/{{TAB}}, plus a trailing Enter for press_enter. */
    val segments: List<PasteSegment> = emptyList(),
    /** [finalText] without paste markers (history, copy-last). */
    val plainText: String = "",
    /** EMPTY only: no_audio | no_speech | blank_asr | hallucination | only_filler. */
    val emptyReason: String? = null,
    val route: RouteInfo? = null,
    /** D28: the LLM step's usage (engine, model, cost, ttft); null = LLM off, skipped or refused up front. */
    val llm: CleanUsage? = null,
) {
    /** What to paste: [segments] when the core sent them, else the pre-core-p1 fields. */
    fun pastePlan(): List<PasteSegment> = segments.ifEmpty {
        buildList {
            if (finalText.isNotBlank()) add(PasteSegment.Text(finalText))
            if (pressEnter && isNotEmpty()) add(PasteSegment.Key(PasteKey.ENTER))
        }
    }

    /** History / copy text: [plainText], else [finalText] (older libs). */
    val historyText: String get() = plainText.ifEmpty { finalText }
}

data class LearnCandidate(val original: String, val corrected: String)
