package dev.flowclone.core

/**
 * In-process stand-in for the Rust core: returns [transcript] for any session that
 * received non-silent audio. Used by unit tests and by the "mock engine" debug setting
 * so the shell (bubble → insertion) can be exercised on a device without libflow_ffi.so.
 */
class FakeFlowCore(@Volatile var transcript: String = "") : FlowCore {
    @Volatile private var active = false
    @Volatile private var heardSignal = false
    @Volatile var samplesPushed = 0; private set
    @Volatile var lastConfig: SessionConfig? = null; private set
    @Volatile var lastContext: FieldContext? = null; private set
    @Volatile var unloaded = false; private set

    override fun startSession(cfg: SessionConfig) {
        if (unloaded) throw CoreException("unloaded")
        lastConfig = cfg; active = true; heardSignal = false; samplesPushed = 0; lastContext = null
    }

    override fun pushAudio(pcm: ShortArray, length: Int): Boolean {
        if (!active) return false
        samplesPushed += length
        for (i in 0 until length) if (kotlin.math.abs(pcm[i].toInt()) > 184) { heardSignal = true; break }
        return true
    }

    override fun setContext(ctx: FieldContext) { lastContext = ctx }

    override fun partial(): String? = null

    override fun commit(): CommitResult {
        if (!active) return CommitResult(CommitStatus.ERROR, error = "no active session")
        active = false
        val audioMs = samplesPushed / 16L
        return if (!heardSignal || transcript.isBlank()) CommitResult(CommitStatus.EMPTY, timings = Timings(audioMs = audioMs))
        else CommitResult(CommitStatus.RAW_TRANSCRIPT, asrText = transcript, finalText = transcript, timings = Timings(audioMs = audioMs))
    }

    override fun cancel() { active = false }

    override fun learnEdit(pasted: String, edited: String) = emptyList<LearnCandidate>()

    override fun unload() { active = false; unloaded = true }
}
