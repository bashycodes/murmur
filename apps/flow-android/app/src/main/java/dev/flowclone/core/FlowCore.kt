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

class CoreException(message: String) : Exception(message)

/** crates/flow-core `ModelConfig`. [mockText] selects the core's `Mock` ASR backend. */
data class ModelConfig(
    val asrPath: String,
    val llmPath: String? = null,
    val threads: Int = 4,
    val mockText: String? = null,
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
)

data class LearnCandidate(val original: String, val corrected: String)
