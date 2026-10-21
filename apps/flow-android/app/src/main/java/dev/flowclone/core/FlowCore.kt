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

    /** `commit(h)`: blocks until ASR (+cleanup) finishes. Never throws; errors are [CommitStatus.ERROR]. */
    fun commit(): CommitResult

    /** `cancel(h)`. */
    fun cancel()

    /** `learn_edit(h, pasted, edited)`. */
    fun learnEdit(pasted: String, edited: String): List<LearnCandidate>

    /** `unload(h)`: frees models. The instance is unusable afterwards. */
    fun unload()
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

    /** User-facing text for the bubble / main screen (§5 "Errors in normal UI"). */
    fun userMessage(kind: String?, message: String): String = when (kind) {
        NO_ENGINE_ENABLED -> "Dictation disabled — enable server or client in Developer settings"
        SERVER_UNAVAILABLE -> "Server unreachable (${message.substringAfter(':', message).trim()})"
        SERVER_NOT_CONFIGURED -> "No server configured — set the host in Developer settings"
        LOCAL_NOT_READY -> "On-device speech model is not ready"
        "model_not_found" -> "Speech model file is missing (${message.substringAfter(':').trim()}). Re-import it."
        else -> message
    }
}

/** `ModelConfig.asr_engine`: whisper.cpp (any language) or Parakeet-TDT (English, D12). */
enum class AsrEngine(val wire: String) { WHISPER("whisper"), PARAKEET("parakeet") }

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
)

enum class CleanupLevel(val wire: String) { NONE("none"), LIGHT("light"), MEDIUM("medium"), HIGH("high") }

enum class Style(val wire: String) { FORMAL("formal"), CASUAL("casual"), VERY_CASUAL("very_casual"), EXCITED("excited") }

data class Styles(
    val personal: Style = Style.FORMAL,
    val work: Style = Style.FORMAL,
    val email: Style = Style.FORMAL,
    val other: Style = Style.FORMAL,
)

/** Shared dictionary/snippet export row `{word, replacement, isSnippet}` (§9). */
data class DictEntry(val word: String, val replacement: String? = null, val isSnippet: Boolean = false)

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
    /** "local" | "remote" */
    val engine: String,
    /** connect_failed, auth_failed, proto_mismatch, busy, timeout, disconnected, circuit_open, server_not_configured, local_not_ready */
    val fallback: String? = null,
    /** "host:port" */
    val server: String? = null,
    val rttMs: Long? = null,
    val queueMs: Long? = null,
    val serverCore: String? = null,
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
