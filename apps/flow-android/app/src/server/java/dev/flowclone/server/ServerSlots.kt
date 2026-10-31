package dev.flowclone.server

import dev.flowclone.data.HebrewModels
import dev.flowclone.data.ModelPages
import java.io.File

/**
 * D41: where a library model goes on the phone. Mirrors the Windows rules
 * (apps/flow-windows/src/server_library.rs `plan`): a model tagged with a catalogue id this
 * build knows is that catalogue model and must match its built-in pins; any other model is
 * custom, checked against the server's SHA-256, and its `kind` + `engine` pick the slot.
 * Android differences: there is no language-ID slot (the phone loads one model for the
 * dictation language, which comes from the keyboard or the settings, D37), and single-file
 * custom models download only their model file (less data on a cellular link).
 */
enum class Slot(val screen: ModelPages.Screen, val label: String) {
    /** whisper.cpp ggml file → `files/models/`, the speech model. */
    STT_WHISPER(ModelPages.Screen.WHISPER, "Speech model (whisper)"),
    /** sherpa-onnx Parakeet folder → `files/parakeet/`. */
    STT_PARAKEET(ModelPages.Screen.PARAKEET, "Speech model (Parakeet)"),
    /** sherpa-onnx streaming transducer folder → `files/hebrew/`, the Hebrew route. */
    HEBREW_ZIPFORMER(ModelPages.Screen.HEBREW, "Hebrew model (streaming)"),
    /** Parakeet folder → `files/hebrew/`, the Hebrew route. */
    HEBREW_PARAKEET(ModelPages.Screen.HEBREW, "Hebrew model (Parakeet)"),
    /** llama.cpp GGUF → `files/llm/`, AI cleanup. */
    LLM(ModelPages.Screen.LLM, "AI cleanup model");

    /** Single-file slots keep one file with this extension; the others are folders. */
    val fileExt: String? get() = when (this) {
        STT_WHISPER -> ".bin"
        LLM -> ".gguf"
        else -> null
    }
}

/** A catalogue model this build knows, with the pins its server copy must match. */
data class CatalogueModel(val id: String, val title: String, val slot: Slot, val dest: String, val pins: List<LibraryFile>)

/**
 * The catalogue ids (same as Windows' `model_download::CATALOG`) and their pinned files.
 * The Hebrew pins are [HebrewModels]' (import uses them too); ServerCatalogTest checks every
 * pin against the Windows catalogue, so the two apps can't drift apart. The Hebrew entries'
 * language-ID extra is not fetched (no slot on Android).
 */
object ServerCatalog {
    private fun pin(name: String, size: Long, sha: String) = LibraryFile(name, size, sha)
    private fun he(k: HebrewModels.Kind) = k.pins.map { pin(it.name, it.size, it.sha256) }

    val ALL: List<CatalogueModel> = listOf(
        CatalogueModel(
            "parakeet-tdt-0.6b-v2-int8", "Parakeet TDT 0.6B", Slot.STT_PARAKEET, "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
            listOf(
                pin("encoder.int8.onnx", 652_184_296, "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab"),
                pin("decoder.int8.onnx", 7_257_753, "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e"),
                pin("joiner.int8.onnx", 1_739_080, "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2"),
                pin("tokens.txt", 9_384, "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d"),
            ),
        ),
        CatalogueModel(
            "whisper-base", "Whisper base", Slot.STT_WHISPER, "ggml-base.bin",
            listOf(pin("ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe")),
        ),
        CatalogueModel(
            "whisper-small-q5_1", "Whisper small", Slot.STT_WHISPER, "ggml-small-q5_1.bin",
            listOf(pin("ggml-small-q5_1.bin", 190_085_487, "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb")),
        ),
        CatalogueModel("zipformer-he-int8", HebrewModels.Kind.ZIPFORMER.label, Slot.HEBREW_ZIPFORMER, HebrewModels.Kind.ZIPFORMER.dirName, he(HebrewModels.Kind.ZIPFORMER)),
        CatalogueModel("parakeet-tdt-0.6b-v3-he-int8", HebrewModels.Kind.PARAKEET.label, Slot.HEBREW_PARAKEET, HebrewModels.Kind.PARAKEET.dirName, he(HebrewModels.Kind.PARAKEET)),
        CatalogueModel(
            "qwen2.5-0.5b-instruct-q4_k_m", "Qwen2.5 0.5B Instruct", Slot.LLM, "qwen2.5-0.5b-instruct-q4_k_m.gguf",
            listOf(pin("qwen2.5-0.5b-instruct-q4_k_m.gguf", 491_400_032, "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db")),
        ),
        CatalogueModel(
            "qwen2.5-1.5b-instruct-q4_k_m", "Qwen2.5 1.5B Instruct", Slot.LLM, "qwen2.5-1.5b-instruct-q4_k_m.gguf",
            listOf(pin("qwen2.5-1.5b-instruct-q4_k_m.gguf", 1_117_320_736, "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e")),
        ),
        CatalogueModel(
            "llama-3.2-3b-instruct-q4_k_m", "Llama 3.2 3B Instruct", Slot.LLM, "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
            listOf(pin("Llama-3.2-3B-Instruct-Q4_K_M.gguf", 2_019_377_696, "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff")),
        ),
    )

    fun entry(id: String?): CatalogueModel? = id?.let { i -> ALL.firstOrNull { it.id == i } }
}

/** The app folders each slot installs into (`FlowApp.Engine.*Dir()`). */
data class ModelDirs(val models: File, val parakeet: File, val hebrew: File, val llm: File) {
    fun of(slot: Slot): File = when (slot) {
        Slot.STT_WHISPER -> models
        Slot.STT_PARAKEET -> parakeet
        Slot.HEBREW_ZIPFORMER, Slot.HEBREW_PARAKEET -> hebrew
        Slot.LLM -> llm
    }
}

/** One file to fetch: [file] is what it must match (a pin, or the server's listing), saved as [dest]. */
data class Fetch(val file: LibraryFile, val dest: File) {
    val part: File get() = File(dest.parentFile, dest.name + ".part")
}

/** How a library model installs here: [target] is what the settings point at (a file or a folder). */
data class Plan(val slot: Slot, val catalogue: CatalogueModel?, val target: File, val fetches: List<Fetch>) {
    val bytes: Long get() = fetches.sumOf { it.file.size }

    /** Every file is in place with its size (each was verified before it was renamed there). */
    val installed: Boolean get() = fetches.all { it.dest.isFile && it.dest.length() == it.file.size }

    /** Bytes already on the phone (finished files and `.part` prefixes: a download resumes after them). */
    val bytesOnDisk: Long get() = fetches.sumOf { f ->
        when {
            f.dest.isFile && f.dest.length() == f.file.size -> f.file.size
            f.part.isFile -> f.part.length().coerceAtMost(f.file.size)
            else -> 0L
        }
    }
}

object ServerSlots {
    /** Folder name of custom model [id] inside its slot's folder. */
    fun customName(id: String) = "server-$id"

    /** Which model screen lists [m] (also the unusable ones, with their reason); null = none. */
    fun screenOf(m: LibraryModel): ModelPages.Screen? {
        ServerCatalog.entry(m.catalogueId)?.let { return it.slot.screen }
        return when (m.kind) {
            ModelKind.STT -> if (m.engine == "parakeet") ModelPages.Screen.PARAKEET else ModelPages.Screen.WHISPER
            ModelKind.HEBREW, ModelKind.LID -> ModelPages.Screen.HEBREW
            ModelKind.LLM -> ModelPages.Screen.LLM
            ModelKind.VAD, null -> ModelPages.Screen.WHISPER
        }
    }

    /** The slot and files of [m], or why it can't be used in this build ([local] = the build has on-device engines). */
    fun plan(m: LibraryModel, dirs: ModelDirs, local: Boolean): Result<Plan> = runCatching {
        if (!local) error("this build has no on-device engine (it dictates on your server)")
        ServerCatalog.entry(m.catalogueId)?.let { e ->
            for (p in e.pins) {
                val s = m.files.firstOrNull { it.name == p.name } ?: error("the server's copy has no ${p.name}")
                if (s.size != p.size || !s.sha256.equals(p.sha256, ignoreCase = true)) error("the server's ${p.name} differs from the fingerprint built into this version")
            }
            val dir = dirs.of(e.slot)
            return@runCatching if (e.slot.fileExt != null) {
                val t = File(dir, e.dest)
                Plan(e.slot, e, t, listOf(Fetch(e.pins.single(), t)))
            } else {
                val t = File(dir, e.dest)
                Plan(e.slot, e, t, e.pins.map { Fetch(it, File(t, it.name)) })
            }
        }
        val engine = m.engine.orEmpty()
        val slot = when {
            m.kind == null -> error("this version doesn't know the kind \"${m.kindWire}\"")
            m.kind == ModelKind.VAD -> error("this app has no slot for a voice-activity model")
            m.kind == ModelKind.LID -> error("the phone has no language-detection slot: dictation follows the keyboard or the language you set")
            engine.isEmpty() -> error("its tag has no engine (add engine = \"whisper\", \"parakeet\", … to model.toml)")
            m.kind == ModelKind.STT && engine == "whisper" -> Slot.STT_WHISPER
            m.kind == ModelKind.STT && engine == "parakeet" -> Slot.STT_PARAKEET
            m.kind == ModelKind.STT && engine == "sherpa_online" -> error("this app runs streaming (sherpa_online) models only as the Hebrew model")
            m.kind == ModelKind.HEBREW && engine == "sherpa_online" -> Slot.HEBREW_ZIPFORMER
            m.kind == ModelKind.HEBREW && engine == "parakeet" -> Slot.HEBREW_PARAKEET
            m.kind == ModelKind.HEBREW -> error("the Hebrew model must be sherpa_online or parakeet, not $engine")
            m.kind == ModelKind.LLM && engine == "gguf" -> Slot.LLM
            else -> error("this app can't load the $engine engine")
        }
        val dir = dirs.of(slot)
        val ext = slot.fileExt
        if (ext != null) {
            val f = m.files.filter { it.name.endsWith(ext) }.singleOrNull() ?: error("expected exactly one $ext file")
            val base = f.name.substringAfterLast('/')
            val t = File(dir, if (base.startsWith(m.id)) base else "${m.id}-$base")
            Plan(slot, null, t, listOf(Fetch(f, t)))
        } else {
            fun top(pre: String) = m.files.any { !it.name.contains('/') && it.name.startsWith(pre) && it.name.endsWith(".onnx") }
            if (!(m.files.any { it.name == "tokens.txt" } && top("encoder") && top("decoder") && top("joiner"))) {
                error("expected encoder, decoder and joiner .onnx files and tokens.txt")
            }
            val t = File(dir, customName(m.id))
            Plan(slot, null, t, m.files.map { Fetch(it, it.name.split('/').fold(t) { d, part -> File(d, part) }) })
        }
    }
}
