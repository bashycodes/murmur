package dev.flowclone.data

import dev.flowclone.core.CoreException
import dev.flowclone.core.FlowCore
import dev.flowclone.core.FlowCoreLoader
import dev.flowclone.core.ModelConfig

/** Model-file sniffing for the SAF import screens (reject the wrong kind before copying 1 GB). */
object ModelFiles {
    enum class Kind { ASR_GGML, LLM_GGUF, UNKNOWN }

    data class Recommendation(val file: String, val sizeMb: Int, val note: String, val url: String)

    /** Qwen2.5-Instruct Q4_K_M: the pair flow-core's llm_e2e measures (docs/core-architecture.md). */
    val LLM_RECOMMENDATIONS = listOf(
        Recommendation(
            "qwen2.5-0.5b-instruct-q4_k_m.gguf", 490,
            "Fastest. Fixes punctuation and casing; misses some backtracking (\"scratch that\").",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        ),
        Recommendation(
            "qwen2.5-1.5b-instruct-q4_k_m.gguf", 1120,
            "Best quality (8/8 on the host test set) but about 3× slower. Flagship phones only.",
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        ),
    )

    /** whisper.cpp files start with the ggml magic 0x67676d6c (little-endian "lmgg"); llama.cpp's with "GGUF". */
    fun kindOf(header: ByteArray): Kind {
        if (header.size < 4) return Kind.UNKNOWN
        val m = String(header, 0, 4, Charsets.ISO_8859_1)
        return when (m) {
            "GGUF" -> Kind.LLM_GGUF
            "lmgg" -> Kind.ASR_GGML
            else -> Kind.UNKNOWN
        }
    }

    /** Null when [actual] is what the screen wants, else a message for the user. */
    fun mismatch(expected: Kind, actual: Kind): String? = when {
        expected == actual -> null
        actual == Kind.LLM_GGUF -> "This is a GGUF language model. Import it under AI cleanup, not as the speech model."
        actual == Kind.ASR_GGML -> "This is a whisper.cpp speech model (ggml-*.bin). Import it as the speech model."
        expected == Kind.LLM_GGUF -> "Not a GGUF file. AI cleanup needs a llama.cpp .gguf model, e.g. qwen2.5-0.5b-instruct-q4_k_m.gguf."
        else -> "Not a whisper.cpp ggml model. Speech models are ggml-*.bin files."
    }
}

/**
 * What to load for the current settings, and loading it with an LLM fallback: if the GGUF or
 * the flow-llm plugin fails, dictation still works with ASR only and the UI says why.
 */
object EnginePlan {
    sealed interface LlmStatus {
        data object Off : LlmStatus
        data object NoModel : LlmStatus
        data class Loaded(val name: String) : LlmStatus
        data class Failed(val message: String) : LlmStatus
    }

    sealed interface Plan {
        data object Mock : Plan
        data class Missing(val message: String) : Plan
        data class Native(val config: ModelConfig, val llm: LlmStatus) : Plan
    }

    data class Loaded(val core: FlowCore?, val description: String, val llm: LlmStatus, val error: String? = null)

    fun plan(s: Settings, asrPath: String?, llmPath: String?, exists: (String) -> Boolean, threads: Int, llmLib: String? = null): Plan {
        if (s.mockEngine) return Plan.Mock
        if (asrPath == null || !exists(asrPath)) return Plan.Missing("No speech model imported. Open Flow Clone → Speech model.")
        val llm = when {
            !s.llmEnabled -> LlmStatus.Off
            llmPath == null || !exists(llmPath) -> LlmStatus.NoModel
            else -> LlmStatus.Loaded(llmPath.substringAfterLast('/'))
        }
        val cfg = ModelConfig(asrPath = asrPath, llmPath = llmPath.takeIf { llm is LlmStatus.Loaded }, threads = threads, llmLib = llmLib)
        return Plan.Native(cfg, llm)
    }

    fun load(p: Plan.Native, loader: FlowCoreLoader): Loaded {
        val name = p.config.asrPath.substringAfterLast('/')
        if (p.config.llmPath != null) {
            try {
                return Loaded(loader.loadModels(p.config), name, p.llm)
            } catch (e: CoreException) {
                val asrOnly = p.config.copy(llmPath = null, llmLib = null)
                return try {
                    Loaded(loader.loadModels(asrOnly), name, LlmStatus.Failed(e.message ?: "LLM failed to load"))
                } catch (e2: CoreException) {
                    Loaded(null, name, LlmStatus.Failed(e.message ?: "LLM failed to load"), e2.message ?: "Speech engine failed to load")
                }
            }
        }
        return try {
            Loaded(loader.loadModels(p.config), name, p.llm)
        } catch (e: CoreException) {
            Loaded(null, name, p.llm, e.message ?: "Speech engine failed to load")
        }
    }
}
