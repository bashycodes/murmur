package dev.flowclone.data

import dev.flowclone.core.RouterErrors

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CoreException
import dev.flowclone.core.FlowCore
import dev.flowclone.core.FlowCoreLoader
import dev.flowclone.core.ModelConfig
import dev.flowclone.core.ParakeetDecoding

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

    const val UNREADABLE = "Could not read the file."

    /**
     * The import screen's accept/reject decision: null = import it, else the message to show.
     * [header] null = the file could not be opened or read at all.
     */
    fun rejection(expected: Kind, header: ByteArray?): String? =
        if (header == null) UNREADABLE else mismatch(expected, kindOf(header))

    /** Up to [n] leading bytes; loops because one `read()` may return fewer bytes than asked. */
    fun readHeader(input: java.io.InputStream, n: Int = 4): ByteArray {
        val buf = ByteArray(n)
        var got = 0
        while (got < n) {
            val r = input.read(buf, got, n - got)
            if (r < 0) break
            got += r
        }
        return if (got == n) buf else buf.copyOf(got)
    }
}

/**
 * The on-device ASR that actually loaded (may differ from the selected one: Parakeet failed →
 * whisper). [model] = the whisper file name or the Parakeet folder name.
 */
data class LoadedAsr(val engine: AsrEngine, val model: String)

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
        /** [whisperFallback]: Parakeet selected; load whisper instead if Parakeet fails (D12 notes, point 5). */
        data class Native(val config: ModelConfig, val llm: LlmStatus, val whisperFallback: ModelConfig? = null) : Plan
    }

    /** [asr]: what loaded (null when [core] is null). */
    data class Loaded(val core: FlowCore?, val description: String, val llm: LlmStatus, val error: String? = null, val asr: LoadedAsr? = null)

    /**
     * [parakeetPath]: imported sherpa-onnx model directory (`isModelDir` checks it). Parakeet is
     * used when selected and present; otherwise whisper ([asrPath]) as before.
     */
    fun plan(
        s: Settings, asrPath: String?, llmPath: String?, exists: (String) -> Boolean, threads: Int, llmLib: String? = null,
        parakeetPath: String? = null, isModelDir: (String) -> Boolean = { false },
        /** D22: greedy on the phone (one recognizer, ≈ 0.6 GB less RAM) unless Developer settings override it. */
        parakeetDecoding: ParakeetDecoding = ParakeetDecoding.GREEDY,
    ): Plan {
        if (s.mockEngine) return Plan.Mock
        val whisperOk = asrPath != null && exists(asrPath)
        val parakeetOk = s.asrEngine == AsrEngine.PARAKEET && parakeetPath != null && isModelDir(parakeetPath)
        if (!whisperOk && !parakeetOk) {
            return Plan.Missing(
                if (s.asrEngine == AsrEngine.PARAKEET) "No Parakeet model folder imported. Open Flow Clone → Speech model."
                else "No speech model imported. Open Flow Clone → Speech model.",
            )
        }
        val llm = when {
            !s.llmEnabled -> LlmStatus.Off
            llmPath == null || !exists(llmPath) -> LlmStatus.NoModel
            else -> LlmStatus.Loaded(llmPath.substringAfterLast('/'))
        }
        val llmUsed = llmPath.takeIf { llm is LlmStatus.Loaded }
        val whisper = if (whisperOk) ModelConfig(asrPath = asrPath!!, llmPath = llmUsed, threads = threads, llmLib = llmLib) else null
        if (parakeetOk) {
            val cfg = ModelConfig(asrPath = parakeetPath!!, llmPath = llmUsed, threads = threads, llmLib = llmLib, asrEngine = AsrEngine.PARAKEET, parakeetDecoding = parakeetDecoding)
            return Plan.Native(cfg, llm, whisperFallback = whisper)
        }
        return Plan.Native(whisper!!, llm)
    }

    fun load(p: Plan.Native, loader: FlowCoreLoader): Loaded {
        val first = loadOne(p, loader)
        val fb = p.whisperFallback
        if (first.core != null || fb == null) return first
        // Parakeet failed (RAM, missing files, a build without the feature): dictate with whisper.
        val r = loadOne(Plan.Native(fb, p.llm), loader)
        return r.copy(description = "${r.description} (Parakeet failed: ${first.error})")
    }

    private fun loadOne(p: Plan.Native, loader: FlowCoreLoader): Loaded {
        val asr = LoadedAsr(p.config.asrEngine, p.config.asrPath.trimEnd('/').substringAfterLast('/'))
        val name = (if (p.config.asrEngine == AsrEngine.PARAKEET) "Parakeet " else "") + asr.model
        val r = loadOneCore(p, loader, name)
        return if (r.core != null) r.copy(asr = asr) else r
    }

    private fun loadOneCore(p: Plan.Native, loader: FlowCoreLoader, name: String): Loaded {
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

/** Parakeet (sherpa-onnx) model directory import: which picked files to keep (D12; ~661 MB int8). */
object ParakeetFiles {
    /** Files the core needs, by role; `*.int8.onnx` preferred over full precision. */
    fun select(names: Collection<String>): Map<String, String>? {
        fun pick(role: String): String? {
            val c = names.filter { it.startsWith(role) && it.endsWith(".onnx") }
            return c.firstOrNull { it.endsWith(".int8.onnx") } ?: c.firstOrNull()
        }
        val out = linkedMapOf<String, String>()
        for (r in listOf("encoder", "decoder", "joiner")) out[r] = pick(r) ?: return null
        out["tokens"] = names.firstOrNull { it == "tokens.txt" } ?: return null
        return out
    }

    fun missing(names: Collection<String>): List<String> = buildList {
        for (r in listOf("encoder", "decoder", "joiner")) if (names.none { it.startsWith(r) && it.endsWith(".onnx") }) add("$r*.onnx")
        if ("tokens.txt" !in names) add("tokens.txt")
    }

    fun isModelDir(dir: java.io.File) = dir.isDirectory && select(dir.list()?.toList().orEmpty()) != null
}

/** `nativeOpenRouter`: wraps an optional local core in flow_router::Router (takes ownership of [local]). */
fun interface RouterOpener {
    /** @throws CoreException with the router's error kind (`no_engine_enabled`, `bad_config`, …). */
    fun open(routerConfigJson: String, local: FlowCore?): FlowCore
}

/**
 * D17 §6.2 Kotlin flow: load the local engine (EnginePlan, with its LLM / Parakeet fallbacks)
 * when this build has one and the client route needs it, then hand it to the router; every
 * session call goes to the router. The mock engine stays a direct FakeFlowCore.
 */
object EngineRoute {
    sealed interface Plan {
        data object Mock : Plan
        /** Nothing can dictate (both routes off, or client-only without a model). */
        data class Unavailable(val message: String, val kind: String?) : Plan
        data class Routed(
            val local: EnginePlan.Plan.Native?,
            /** Why the client route has no local engine (shown on the main screen). */
            val localNote: String?,
            val routerConfigJson: String,
            val dev: DevSettings,
        ) : Plan
    }

    data class Assembled(
        val core: FlowCore?,
        val description: String,
        val llm: EnginePlan.LlmStatus,
        val error: String? = null,
        val errorKind: String? = null,
        /** The local ASR handed to the router (null: none loaded / not needed). */
        val asr: LoadedAsr? = null,
    )

    /** Load local models at startup? (§1.3 `keep_local_warm`: also when the server is preferred.) */
    fun needsLocal(eff: DevSettings, caps: BuildCaps): Boolean =
        caps.local && eff.allowClient && (eff.keepLocalWarm || !eff.allowServer || eff.prefer == DevSettings.Prefer.CLIENT)

    fun plan(local: EnginePlan.Plan, dev: DevSettings, caps: BuildCaps): Plan {
        if (local is EnginePlan.Plan.Mock) return Plan.Mock
        val eff = dev.effective(caps)
        if (eff.bothOff) {
            return Plan.Unavailable(RouterErrors.userMessage("no_engine_enabled", ""), "no_engine_enabled")
        }
        val json = eff.toRouterConfigJson().toString()
        if (!needsLocal(eff, caps)) return Plan.Routed(null, null, json, eff)
        return when (local) {
            is EnginePlan.Plan.Native -> Plan.Routed(local, null, json, eff)
            is EnginePlan.Plan.Missing ->
                if (eff.allowServer || eff.cloudRouted) Plan.Routed(null, local.message, json, eff)
                else Plan.Unavailable(local.message, "model_not_found")
            EnginePlan.Plan.Mock -> Plan.Mock
        }
    }

    fun assemble(p: Plan.Routed, loader: FlowCoreLoader, opener: RouterOpener): Assembled {
        var localCore: FlowCore? = null
        val desc = mutableListOf<String>()
        var llm: EnginePlan.LlmStatus = EnginePlan.LlmStatus.Off
        var localErr: String? = p.localNote
        var asr: LoadedAsr? = null
        if (p.local != null) {
            val r = EnginePlan.load(p.local, loader)
            llm = r.llm
            localCore = r.core
            asr = r.asr
            if (r.core == null) localErr = r.error ?: "Speech engine failed to load"
            else desc += "client: ${r.description}"
        }
        if (localCore == null && !p.dev.allowServer && !p.dev.cloudRouted) {
            return Assembled(null, desc.joinToString(), llm, localErr ?: "Speech engine failed to load", "load")
        }
        if (p.dev.allowServer) desc.add(if (p.dev.prefer == DevSettings.Prefer.SERVER) 0 else desc.size, "server: ${p.dev.host.ifEmpty { "(not set)" }}:${p.dev.port}")
        if (p.dev.cloudRouted) {
            val name = p.dev.cloud.activeProvider?.displayName ?: "(no provider)"
            desc.add(if (p.dev.cloudFirst) 0 else desc.size, "cloud: $name")
        }
        if (localErr != null) desc += "client unavailable: $localErr"
        return try {
            Assembled(opener.open(p.routerConfigJson, localCore), desc.joinToString(" → "), llm, asr = asr)
        } catch (e: CoreException) {
            // The router took ownership of localCore either way.
            Assembled(null, desc.joinToString(" → "), llm, e.message ?: "router failed", e.kind)
        }
    }
}
