package dev.murmur.core

/**
 * In-process stand-in for the Rust core: returns [transcript] for any session that
 * received non-silent audio. Used by unit tests and by the "mock engine" debug setting
 * so the shell (bubble → insertion) can be exercised on a device without libmurmur_ffi.so.
 */
class FakeMurmurCore(@Volatile var transcript: String = "", private val hasLlm: Boolean = false) : MurmurCore {
    @Volatile private var active = false
    @Volatile private var heardSignal = false
    @Volatile var samplesPushed = 0; private set
    @Volatile var lastConfig: SessionConfig? = null; private set
    @Volatile var lastContext: FieldContext? = null; private set
    @Volatile var unloaded = false; private set

    /** Live route reported by [startSessionInfo] / [poll] while a session is active (tests). */
    @Volatile var route: SessionRoute? = null
    /** Hands-free sessions only: [poll] reports `endpoint_reached` once this many samples arrived (tests). */
    @Volatile var endpointAfterSamples: Int? = null
    /** Tests: reshape the commit result (e.g. add D22 timings or a route). */
    @Volatile var shapeResult: ((CommitResult) -> CommitResult)? = null
    /** Runs inside [startSession] (tests: block it, or record the calling thread). */
    @Volatile var onStart: (() -> Unit)? = null

    override fun startSession(cfg: SessionConfig) {
        if (unloaded) throw CoreException("unloaded")
        BlockingCalls.check("startSession")
        onStart?.invoke()
        lastConfig = cfg; active = true; heardSignal = false; samplesPushed = 0; lastContext = null
    }

    override fun pushAudio(pcm: ShortArray, length: Int): Boolean {
        if (!active) return false
        samplesPushed += length
        for (i in 0 until length) if (kotlin.math.abs(pcm[i].toInt()) > 184) { heardSignal = true; break }
        return true
    }

    override fun setContext(ctx: FieldContext) { lastContext = ctx }

    /** What [partial] returns while a session is active (tests / live-preview demo). */
    @Volatile var partialText: String? = null

    override fun partial(): String? = if (active) partialText else null

    override fun startSessionInfo(cfg: SessionConfig): StartInfo { startSession(cfg); return StartInfo(route = route) }
    /** On-device (the mock engine): a local start is an ordinary start (D55). */
    override fun startSessionLocal(cfg: SessionConfig): StartInfo = startSessionInfo(cfg)

    override fun poll(): CorePoll {
        if (!active) return CorePoll()
        val end = lastConfig?.endpointSilenceMs != null && endpointAfterSamples?.let { samplesPushed >= it } == true
        return CorePoll(partial = partialText, endpointReached = end, route = route)
    }

    override fun commit(): CommitResult {
        BlockingCalls.check("commit")
        if (!active) return CommitResult(CommitStatus.ERROR, error = "no active session")
        active = false
        val audioMs = samplesPushed / 16L
        // Same shape as core-p1: EMPTY carries empty_reason; text results carry segments + plain_text.
        val r = if (!heardSignal || transcript.isBlank()) {
            CommitResult(CommitStatus.EMPTY, timings = Timings(audioMs = audioMs), emptyReason = if (samplesPushed == 0) "no_audio" else "no_speech")
        } else {
            CommitResult(
                CommitStatus.RAW_TRANSCRIPT, asrText = transcript, finalText = transcript, timings = Timings(audioMs = audioMs),
                segments = listOf(PasteSegment.Text(transcript)), plainText = transcript,
            )
        }
        return shapeResult?.invoke(r) ?: r
    }

    override fun cancel() { active = false }

    /** Command Mode (tests): the answer; null = the engine has no AI ([CommandOutcome.noLlm]). */
    @Volatile var commandReply: ((instruction: String, selected: String?) -> CommandOutcome)? = null
    /** Every [command] call as (instruction, selected). */
    val commands: MutableList<Pair<String, String?>> = java.util.Collections.synchronizedList(mutableListOf())

    override fun command(instruction: String, selected: String?): CommandOutcome {
        BlockingCalls.check("command")
        commands += instruction to selected
        return commandReply?.invoke(instruction, selected) ?: CommandOutcome.noLlm
    }

    /** D90 (tests): the translation; null = the engine has no AI ([TranslateOutcome.noLlm]). */
    @Volatile var translateReply: ((text: String, target: String?) -> TranslateOutcome)? = null
    /** Every [translate] call as (text, source, target, installed). */
    val translations: MutableList<List<Any?>> = java.util.Collections.synchronizedList(mutableListOf())

    override fun translate(text: String, source: String?, target: String?, installed: List<String>, dictionary: List<DictEntry>): TranslateOutcome {
        BlockingCalls.check("translate")
        translations += listOf(text, source, target, installed)
        return translateReply?.invoke(text, target) ?: TranslateOutcome.noLlm
    }

    override fun learnEdit(pasted: String, edited: String) = emptyList<LearnCandidate>()

    /** Load 3: [keepLlm] calls, each with whether this engine was still loaded (tests). */
    val keepLlmCalls = java.util.concurrent.CopyOnWriteArrayList<Boolean>()

    override fun keepLlm(): Boolean {
        keepLlmCalls += !unloaded
        return hasLlm && !unloaded
    }

    override fun unload() { active = false; unloaded = true }
}
