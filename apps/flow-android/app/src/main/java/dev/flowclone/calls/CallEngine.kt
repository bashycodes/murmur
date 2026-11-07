package dev.flowclone.calls

import dev.flowclone.FlowApp
import dev.flowclone.core.CleanupLevel
import dev.flowclone.core.CoreException
import dev.flowclone.core.FlowCoreLoader
import dev.flowclone.core.MeetingJob
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.core.NativeMeeting
import dev.flowclone.core.SessionConfig
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.HebrewModels
import dev.flowclone.data.ParakeetFiles
import dev.flowclone.data.Settings
import java.io.File

/**
 * D46: the engine a call transcription runs on. Its own on-device core, loaded for the job and
 * freed with it: never the dictation router (one session at a time, D44) and never the server
 * or a cloud provider (call audio stays on the phone, in every flavour). The model is the one a
 * dictation in [language] would load (the Hebrew model for `he` when installed, else the
 * selected engine), without the LLM: chunks are transcribed raw, with the dictionary.
 */
object CallEngine {
    /** Why this build / phone can't transcribe calls, or null when it can. */
    fun unavailable(app: FlowApp): String? = when {
        !app.caps.local -> "This build has no on-device speech engine, so it can't transcribe recordings."
        else -> null
    }

    fun plan(app: FlowApp, language: String): EnginePlan.Plan {
        val s = app.settings.load().copy(llmEnabled = false, mockEngine = false)
        val hebrew = app.engine.hebrewLanguageModel()
        val threads = (Runtime.getRuntime().availableProcessors() / 2).coerceIn(1, 4)
        return EnginePlan.plan(
            s, app.settings.modelPath, null, { File(it).isFile }, threads,
            parakeetPath = app.settings.parakeetPath, isModelDir = { ParakeetFiles.isModelDir(File(it)) },
            // D68: calls stay greedy (the dictionary beam is measured on dictation only).
            parakeetDecoding = EnginePlan.fixedDecoding(app.dev.parakeetDecodingOverride), language = language, hebrew = hebrew,
        )
    }

    /** The session every chunk runs with (`flow_core::meeting::chunk_session` opens its limits). */
    fun session(s: Settings, dictionary: List<dev.flowclone.core.DictEntry>, language: String): SessionConfig =
        s.toSessionConfig(dictionary, language = language).copy(
            cleanupLevel = CleanupLevel.NONE, rawMode = true, partialIntervalMs = null, segmentPauseMs = null, endpointSilenceMs = null,
        )

    /** Blocking (model load): the job's engine + meeting transcriber. @throws CoreException */
    fun open(app: FlowApp, language: String, loader: FlowCoreLoader = NativeFlowCore): MeetingJob {
        unavailable(app)?.let { throw CoreException(it) }
        runCatching { android.system.Os.setenv("TMPDIR", app.cacheDir.path, true) }
        val p = when (val plan = plan(app, language)) {
            is EnginePlan.Plan.Native -> plan
            is EnginePlan.Plan.Missing -> throw CoreException(plan.message, "model_not_found")
            EnginePlan.Plan.Mock -> throw CoreException("The mock engine can't transcribe recordings.")
        }
        val loaded = EnginePlan.load(p, loader)
        val core = loaded.core as? NativeFlowCore ?: throw CoreException(loaded.error ?: "The speech model failed to load")
        val s = app.settings.load()
        return NativeMeeting.open(core, session(s, app.dictionary.all(), language))
    }

    /** The page's default language: Hebrew when dictation is in Hebrew, else English. */
    fun defaultLanguage(app: FlowApp): String = if (HebrewModels.isHebrew(app.effectiveLanguage())) HebrewModels.LANGUAGE else "en"
}
