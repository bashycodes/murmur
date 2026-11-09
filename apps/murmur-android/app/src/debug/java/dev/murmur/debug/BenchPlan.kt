package dev.murmur.debug

import dev.murmur.core.AsrEngine
import dev.murmur.core.CleanupLevel
import dev.murmur.core.LanguageModel
import dev.murmur.core.ModelConfig
import dev.murmur.core.ParakeetDecoding
import dev.murmur.core.SessionConfig
import dev.murmur.data.Settings
import java.io.File

/**
 * Debug builds only: what the on-device benchmark ([BenchActivity], tools/probes/android_bench)
 * runs. Models are read in place from the app's private storage, with the same [ModelConfig] /
 * [SessionConfig] the app builds for a dictation (EngineLogic.plan, Settings.toSessionConfig).
 */
object BenchPlan {
    /** One on-device ASR model; [set] = the fixture set it is scored on (`en` / `he`). */
    data class Stt(val key: String, val set: String, val engine: AsrEngine, val path: String)

    val STT = listOf(
        Stt("parakeet-en", "en", AsrEngine.PARAKEET, "parakeet/parakeet-unified-en-0.6b-int8"),
        Stt("whisper-base", "en", AsrEngine.WHISPER, "models/ggml-base-q5_1.bin"),
        Stt("zipformer-he", "he", AsrEngine.SHERPA_ONLINE, "hebrew/zipformer-he"),
        Stt("parakeet-he", "he", AsrEngine.PARAKEET, "hebrew/parakeet-v3-he"),
    )

    /** On-device cleanup LLMs (llama.cpp GGUF under files/). */
    val LLM = linkedMapOf(
        "qwen1.5b" to "llm/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "qwen0.5b" to "llm/qwen2.5-0.5b-instruct-q4_k_m.gguf",
    )

    val MODES = listOf("prod", "v4c")

    /** The app's thread rule (MurmurApp.load). */
    fun appThreads(cores: Int = Runtime.getRuntime().availableProcessors()) = (cores / 2).coerceIn(1, 4)

    /**
     * As EngineLogic.plan builds it: Parakeet greedy (the phone default, D22); Hebrew models as a
     * `he` language entry loaded alone (`load_language`, H5); no LLM.
     */
    fun modelConfig(m: Stt, filesDir: File, threads: Int): ModelConfig {
        val path = File(filesDir, m.path).path
        return when (m.set) {
            "he" -> {
                val lm = LanguageModel(m.engine, path, if (m.engine == AsrEngine.PARAKEET) ParakeetDecoding.GREEDY else ParakeetDecoding.AUTO)
                ModelConfig(asrPath = path, threads = threads, asrEngine = m.engine, languages = mapOf("he" to lm), loadLanguage = "he")
            }
            else -> ModelConfig(
                asrPath = path, threads = threads, asrEngine = m.engine,
                parakeetDecoding = if (m.engine == AsrEngine.PARAKEET) ParakeetDecoding.GREEDY else ParakeetDecoding.AUTO,
            )
        }
    }

    /**
     * A dictation session in [language] with the LLM off (Settings.toSessionConfig with "Clean up
     * with local AI" off): VAD, hallucination guard and the deterministic formatting stay on, so
     * `final_text` is what the app would paste; `asr_text` is the raw transcript WER is scored on.
     */
    fun sttSession(language: String, segmentPauseMs: Int? = null) = SessionConfig(
        languages = listOf(language),
        cleanupLevel = CleanupLevel.NONE,
        rawMode = true,
        llmTimeoutMs = Settings.ANDROID_LLM_TIMEOUT_MS,
        llmPrefill = false,
        segmentPauseMs = segmentPauseMs,
    )

    /** The LLM-only handle: the core's mock ASR (never used) plus the GGUF, as the app loads it. */
    fun llmConfig(path: String, threads: Int) = ModelConfig(asrPath = "", llmPath = path, threads = threads, mockText = "bench")

    /**
     * An English dictation with "Clean up with local AI" on at the default strength (LIGHT).
     * `prod` keeps the app's LLM timeout; `v4c` gets [v4cTimeoutMs] so the raw answer is never cut
     * (the Linux scorer reports calls over the app's budget; D59: no output guard to apply).
     */
    fun cleanupSession(mode: String, v4cTimeoutMs: Long = 30_000) = SessionConfig(
        languages = listOf("en"),
        cleanupLevel = CleanupLevel.LIGHT,
        llmTimeoutMs = if (mode == "v4c") v4cTimeoutMs else Settings.ANDROID_LLM_TIMEOUT_MS,
    )
}

/**
 * The intent extras of one run (all optional; `am start … --es key value`):
 * `suite` stt|cleanup|all, `stt` model keys, `llm` LLM keys, `modes` prod,v4c, `threads`,
 * `limit` (items per set / cases, 0 = all), `pace` fast|realtime, `segment` (ms, D22 decode while
 * speaking; realtime pace only makes sense with it), `run` (output folder name), `in` (bundle dir).
 */
data class BenchArgs(
    val suite: String = "all",
    val stt: List<String> = BenchPlan.STT.map { it.key },
    val llm: List<String> = BenchPlan.LLM.keys.toList(),
    val modes: List<String> = BenchPlan.MODES,
    val threads: Int = BenchPlan.appThreads(),
    val limit: Int = 0,
    val realtime: Boolean = false,
    val segmentPauseMs: Int? = null,
    val run: String? = null,
    val inDir: String? = null,
) {
    val runStt get() = suite == "all" || suite == "stt"
    val runCleanup get() = suite == "all" || suite == "cleanup"

    companion object {
        private fun list(s: String?) = s?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() }

        /** @throws IllegalArgumentException naming the bad extra. */
        fun parse(get: (String) -> String?): BenchArgs {
            val d = BenchArgs()
            val suite = get("suite") ?: d.suite
            require(suite in setOf("all", "stt", "cleanup")) { "suite must be all, stt or cleanup: $suite" }
            val stt = list(get("stt")) ?: d.stt
            stt.firstOrNull { k -> BenchPlan.STT.none { it.key == k } }?.let { throw IllegalArgumentException("unknown stt model $it") }
            val llm = list(get("llm")) ?: d.llm
            llm.firstOrNull { it !in BenchPlan.LLM }?.let { throw IllegalArgumentException("unknown llm $it") }
            val modes = list(get("modes")) ?: d.modes
            modes.firstOrNull { it !in BenchPlan.MODES }?.let { throw IllegalArgumentException("unknown mode $it") }
            val pace = get("pace") ?: "fast"
            require(pace == "fast" || pace == "realtime") { "pace must be fast or realtime: $pace" }
            fun int(k: String) = get(k)?.let { it.toIntOrNull() ?: throw IllegalArgumentException("$k must be a number: $it") }
            val run = get("run")
            require(run == null || Regex("[A-Za-z0-9._-]{1,64}").matches(run)) { "run must be [A-Za-z0-9._-]: $run" }
            return BenchArgs(
                suite = suite, stt = stt, llm = llm, modes = modes,
                threads = int("threads")?.coerceIn(1, 8) ?: d.threads,
                limit = int("limit")?.coerceAtLeast(0) ?: 0,
                realtime = pace == "realtime",
                segmentPauseMs = int("segment")?.takeIf { it > 0 },
                run = run, inDir = get("in"),
            )
        }
    }
}
