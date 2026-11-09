package dev.murmur.data

import dev.murmur.core.RouterErrors

import dev.murmur.core.AsrEngine
import dev.murmur.core.CoreException
import dev.murmur.core.DictEntry
import dev.murmur.core.MurmurCore
import dev.murmur.core.MurmurCoreLoader
import dev.murmur.core.LanguageModel
import dev.murmur.core.ModelConfig
import dev.murmur.core.ParakeetDecoding

/** Model-file sniffing for the SAF import screens (reject the wrong kind before copying 1 GB). */
object ModelFiles {
    enum class Kind { ASR_GGML, LLM_GGUF, UNKNOWN }

    data class Recommendation(val file: String, val sizeMb: Int, val note: String, val url: String)

    /** Qwen2.5-Instruct Q4_K_M: the pair murmur-core's llm_e2e measures (docs/core-architecture.md). */
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
 * the murmur-llm plugin fails, dictation still works with ASR only and the UI says why.
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

    /**
     * D68: the on-device English Parakeet's search. A Developer [override] wins; otherwise beam
     * search (the dictionary's words become hotwords, as on the PC) once the dictionary has a
     * word, and today's greedy search (D22) while it has none. One recognizer either way, so
     * RAM stays as it was; switching between the two reloads the speech model.
     */
    fun parakeetDecoding(override: ParakeetDecoding?, dictionary: List<DictEntry>): ParakeetDecoding =
        override ?: if (hasHotwords(dictionary)) ParakeetDecoding.BEAM else ParakeetDecoding.GREEDY

    /** D68: Parakeet-he and call transcription: the Developer [override], else greedy (D22). */
    fun fixedDecoding(override: ParakeetDecoding?): ParakeetDecoding = override ?: ParakeetDecoding.GREEDY

    /** The dictionary gives the recognizer hotwords: a word entry (snippets never bias speech). */
    fun hasHotwords(dictionary: List<DictEntry>): Boolean =
        dictionary.any { !it.isSnippet && (it.replacement ?: it.word).isNotBlank() }

    /** [asr]: what loaded (null when [core] is null). */
    data class Loaded(val core: MurmurCore?, val description: String, val llm: LlmStatus, val error: String? = null, val asr: LoadedAsr? = null)

    /**
     * [parakeetPath]: imported sherpa-onnx model directory (`isModelDir` checks it). Parakeet is
     * used when selected and present; otherwise whisper ([asrPath]) as before.
     */
    fun plan(
        s: Settings, asrPath: String?, llmPath: String?, exists: (String) -> Boolean, threads: Int, llmLib: String? = null,
        parakeetPath: String? = null, isModelDir: (String) -> Boolean = { false },
        /** D22 / D68: the English Parakeet's search ([parakeetDecoding]; greedy without a dictionary). */
        parakeetDecoding: ParakeetDecoding = ParakeetDecoding.GREEDY,
        /**
         * H5: [hebrew] = the installed Hebrew model (null = none); used when [language] (the
         * effective dictation language) is Hebrew. Only that model loads (`load_language`).
         */
        language: String? = null,
        hebrew: LanguageModel? = null,
        /** D54 "Keep both languages loaded": with [hebrew] and a default model, load both (no `load_language`). */
        keepBoth: Boolean = false,
    ): Plan {
        if (s.mockEngine) return Plan.Mock
        val whisperOk = whisperOk(asrPath, exists)
        val parakeetOk = parakeetOk(s, parakeetPath, isModelDir)
        val set = LoadSet.of(language, hebrew != null, keepBoth) { whisperOk || parakeetOk }
        val both = set == LoadSet.Both
        val hebrewRoute = (set as? LoadSet.Language)?.code
        if (!whisperOk && !parakeetOk && hebrewRoute == null) {
            return Plan.Missing(
                if (s.asrEngine == AsrEngine.PARAKEET) "No Parakeet model folder imported. Open Murmur → Speech model."
                else "No speech model imported. Open Murmur → Speech model.",
            )
        }
        val llm = when {
            !s.llmEnabled -> LlmStatus.Off
            llmPath == null || !exists(llmPath) -> LlmStatus.NoModel
            else -> LlmStatus.Loaded(llmPath.substringAfterLast('/'))
        }
        val llmUsed = llmPath.takeIf { llm is LlmStatus.Loaded }
        val whisper = if (whisperOk) ModelConfig(asrPath = asrPath!!, llmPath = llmUsed, threads = threads, llmLib = llmLib) else null
        if (hebrewRoute != null) {
            // The default model's fields stay (they are not loaded); whisper is the fallback if the Hebrew model fails.
            val he = hebrew!!
            val base = whisper ?: ModelConfig(asrPath = he.asrPath, llmPath = llmUsed, threads = threads, llmLib = llmLib, asrEngine = he.asrEngine)
            return Plan.Native(base.copy(languages = mapOf(hebrewRoute to he), loadLanguage = hebrewRoute), llm, whisperFallback = whisper)
        }
        // D54: the core loads every language model and picks one per dictation from its language (D33).
        val languages = if (both) mapOf(HebrewModels.LANGUAGE to hebrew!!) else emptyMap()
        if (parakeetOk) {
            val cfg = ModelConfig(asrPath = parakeetPath!!, llmPath = llmUsed, threads = threads, llmLib = llmLib, asrEngine = AsrEngine.PARAKEET, parakeetDecoding = parakeetDecoding, languages = languages)
            return Plan.Native(cfg, llm, whisperFallback = whisper)
        }
        // Both models failing to load together (RAM) falls back to whisper alone, like a failed Hebrew model.
        return Plan.Native(whisper!!.copy(languages = languages), llm, whisperFallback = whisper.takeIf { both })
    }

    private fun whisperOk(asrPath: String?, exists: (String) -> Boolean) = asrPath != null && exists(asrPath)
    private fun parakeetOk(s: Settings, parakeetPath: String?, isModelDir: (String) -> Boolean) =
        s.asrEngine == AsrEngine.PARAKEET && parakeetPath != null && isModelDir(parakeetPath)

    /** D54: a default model [plan] can load (whisper, or Parakeet when selected); same predicates as [plan]. */
    fun defaultInstalled(s: Settings, asrPath: String?, exists: (String) -> Boolean, parakeetPath: String?, isModelDir: (String) -> Boolean): Boolean =
        whisperOk(asrPath, exists) || parakeetOk(s, parakeetPath, isModelDir)

    /** D54: the plan loads the default and the Hebrew model together. */
    fun loadsBoth(c: ModelConfig): Boolean = c.loadLanguage == null && c.languages.isNotEmpty()

    /**
     * What [load] tries before the whisper fallback; [label] names it in the description's
     * "(<label> failed: …)" note, which [EngineMismatch] reads back.
     */
    enum class FirstChoice(val label: String) {
        PARAKEET("Parakeet"), HEBREW("Hebrew model"), BOTH_LANGUAGES("Both languages");

        companion object {
            fun of(c: ModelConfig): FirstChoice = when {
                c.loadLanguage != null -> HEBREW
                loadsBoth(c) -> BOTH_LANGUAGES
                else -> PARAKEET
            }
        }
    }

    /** The note [load] appends when [config]'s load failed with [error] and the fallback loaded. */
    fun failedNote(config: ModelConfig, error: String?): String = "(${FirstChoice.of(config).label} failed: $error)"

    /**
     * D24: the configs [load] would try, in the same order (selected engine with its LLM, then
     * without the LLM, then the whisper fallback likewise), for the router's on-demand loader.
     */
    fun attempts(p: Plan.Native): List<ModelConfig> = buildList {
        for (c in listOfNotNull(p.config, p.whisperFallback)) {
            add(c)
            if (c.llmPath != null) add(c.copy(llmPath = null, llmLib = null))
        }
    }.distinct()

    fun load(p: Plan.Native, loader: MurmurCoreLoader): Loaded {
        val first = loadOne(p, loader)
        val fb = p.whisperFallback
        if (first.core != null || fb == null) return first
        // Parakeet / the Hebrew model failed (RAM, missing files, a build without the feature): dictate with whisper.
        val r = loadOne(Plan.Native(fb, p.llm), loader)
        return r.copy(description = "${r.description} ${failedNote(p.config, first.error)}")
    }

    /** The ASR [c] loads (the routed language model, else the default) as the UI / debug log name it. */
    fun loadedAsrOf(c: ModelConfig): LoadedAsr = c.routed.let { LoadedAsr(it.asrEngine, it.asrPath.trimEnd('/').substringAfterLast('/')) }

    /** The engine the settings ask for: the Hebrew model's while Hebrew routes to it, else the user's choice. */
    fun selectedEngine(s: Settings, local: Plan.Native?): AsrEngine =
        local?.config?.takeIf { it.loadLanguage != null }?.routed?.asrEngine ?: s.asrEngine

    private fun loadOne(p: Plan.Native, loader: MurmurCoreLoader): Loaded {
        val asr = loadedAsrOf(p.config)
        val r = loadOneCore(p, loader, loadName(p.config, asr))
        return if (r.core != null) r.copy(asr = asr) else r
    }

    /** The UI / debug name of what [c] loads: "Hebrew zipformer-he", "Parakeet pk + Hebrew zipformer-he", "base.bin". */
    private fun loadName(c: ModelConfig, asr: LoadedAsr): String {
        val prefix = when {
            c.loadLanguage != null -> "Hebrew "
            asr.engine == AsrEngine.PARAKEET -> "Parakeet "
            else -> ""
        }
        val alongside = c.languages[HebrewModels.LANGUAGE]?.takeIf { loadsBoth(c) }
            ?.let { " + Hebrew " + it.asrPath.trimEnd('/').substringAfterLast('/') }.orEmpty()
        return prefix + asr.model + alongside
    }

    private fun loadOneCore(p: Plan.Native, loader: MurmurCoreLoader, name: String): Loaded {
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

/** D21 event when the on-device engine that loaded is not the one selected in Settings. */
object EngineMismatch {
    /** [EnginePlan.failedNote]'s "(<first choice> failed: <reason>)". */
    private val FIRST_CHOICE_FAILED = Regex(
        """\((?:""" + EnginePlan.FirstChoice.entries.joinToString("|") { Regex.escape(it.label) } + """) failed: (.*?)\)(?: → |$)""",
    )
    private val CLIENT_UNAVAILABLE = Regex("""client unavailable: (.*?)(?: → |$)""")

    /**
     * [local]: the local plan (null = no local engine needed: server / cloud only, mock);
     * [loaded]: what loaded; [description]: the load description (carries the failure reason).
     * Null when the selected engine is the one that loaded.
     */
    fun message(selected: AsrEngine, local: EnginePlan.Plan.Native?, loaded: LoadedAsr?, description: String): String? {
        if (local == null || loaded?.engine == selected) return null
        val reason = FIRST_CHOICE_FAILED.find(description)?.groupValues?.get(1)
            ?: CLIENT_UNAVAILABLE.find(description)?.groupValues?.get(1)
            ?: if (selected == AsrEngine.PARAKEET && local.config.asrEngine != AsrEngine.PARAKEET) "no Parakeet model folder imported" else null
        val what = loaded?.let { "${it.engine.wire} ${it.model}" } ?: "none"
        return "selected ${selected.wire}, loaded $what" + (reason?.let { ": $it" } ?: "")
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

/** `nativeOpenRouter`: wraps an optional local core in murmur_router::Router (takes ownership of [local]). */
fun interface RouterOpener {
    /** @throws CoreException with the router's error kind (`no_engine_enabled`, `bad_config`, …). */
    fun open(routerConfigJson: String, local: MurmurCore?): MurmurCore
}

/**
 * D17 §6.2 Kotlin flow: load the local engine (EnginePlan, with its LLM / Parakeet fallbacks)
 * when this build has one and the client route needs it, then hand it to the router; every
 * session call goes to the router. The mock engine stays a direct FakeMurmurCore.
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
            /**
             * D24: the client route is allowed but not loaded at startup (`keep_local_warm` off):
             * the router loads this on demand when the server can't be reached.
             */
            val onDemand: EnginePlan.Plan.Native? = null,
        ) : Plan
    }

    data class Assembled(
        val core: MurmurCore?,
        val description: String,
        val llm: EnginePlan.LlmStatus,
        val error: String? = null,
        val errorKind: String? = null,
        /** The local ASR handed to the router (null: none loaded / not needed). */
        val asr: LoadedAsr? = null,
        /** D24: why the client route can't serve a fallback (no model imported, load failed); null = it can. */
        val clientNote: String? = null,
        /** D24: what the on-demand loader loads first (debug log `engine`/`model`). */
        val onDemandAsr: LoadedAsr? = null,
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
        if (!needsLocal(eff, caps)) {
            val client = caps.local && eff.allowClient
            val onDemand = (local as? EnginePlan.Plan.Native)?.takeIf { client }
            val note = (local as? EnginePlan.Plan.Missing)?.message?.takeIf { client }
            return Plan.Routed(null, note, json, eff, onDemand)
        }
        return when (local) {
            is EnginePlan.Plan.Native -> Plan.Routed(local, null, json, eff)
            is EnginePlan.Plan.Missing ->
                if (eff.allowServer || eff.cloudRouted) Plan.Routed(null, local.message, json, eff)
                else Plan.Unavailable(local.message, "model_not_found")
            EnginePlan.Plan.Mock -> Plan.Mock
        }
    }

    fun assemble(p: Plan.Routed, loader: MurmurCoreLoader, opener: RouterOpener): Assembled {
        var localCore: MurmurCore? = null
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
            desc.add(if (p.dev.cloudFirst) 0 else desc.size, "cloud: ${CloudText.routeName(p.dev.cloud)}")
        }
        if (localErr != null) desc += "client unavailable: $localErr"
        if (localCore == null && p.onDemand != null) desc += "client: loaded on demand"
        return try {
            val core = opener.open(p.routerConfigJson, localCore)
            // D24: never fail just because the server is down; a failed install is only logged.
            if (localCore == null && p.onDemand != null) runCatching { core.setLocalLoader(EnginePlan.attempts(p.onDemand)) }
            Assembled(core, desc.joinToString(" → "), llm, asr = asr, clientNote = if (localCore == null && p.onDemand == null) localErr else null,
                onDemandAsr = p.onDemand?.takeIf { localCore == null }?.let { EnginePlan.loadedAsrOf(it.config) })
        } catch (e: CoreException) {
            // The router took ownership of localCore either way.
            Assembled(null, desc.joinToString(" → "), llm, e.message ?: "router failed", e.kind)
        }
    }
}
