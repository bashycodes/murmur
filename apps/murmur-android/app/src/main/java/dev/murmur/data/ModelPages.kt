package dev.murmur.data

import java.util.Locale

/**
 * Model import screens (ui/ModelImportActivity) in the Settings look: status card (active model,
 * size, engine), Import as the primary action, recommended files with their download links (the
 * app never downloads: D9), and the imported list. The import itself stays in [ModelFiles].
 */
object ModelPages {
    enum class Screen { WHISPER, PARAKEET, LLM, HEBREW }

    /** [model]: the row in the machine-class table ([ModelFit], D117) that marks it Recommended and fits / doesn't fit. */
    data class Recommended(val file: String, val sizeMb: Int, val speed: String, val quality: String, val note: String, val url: String, val model: ModelFit.Model? = null)

    private const val WHISPER = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
    private const val SHERPA = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
    /** H5: pinned revisions (docs/findings/2026-10-30-hebrew-model-pins.md). */
    const val ZIPFORMER_HE = "https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he/tree/ee769a2a13b887f91572eec349458bfb0fe79e7a/sherpa-onnx"
    const val PARAKEET_HE = "https://huggingface.co/krut42/voice-parakeet-he-int8/tree/39de3b473bed626c7796c156d71adc66161ebfe1"

    fun recommended(s: Screen): List<Recommended> = when (s) {
        Screen.WHISPER -> listOf(
            Recommended("ggml-base.en-q5_1.bin", 60, "Fast", "Good quality", "English only", "$WHISPER/ggml-base.en-q5_1.bin", ModelFit.Model.WHISPER_BASE),
            Recommended("ggml-base-q5_1.bin", 60, "Fast", "Good quality", "Many languages (follows the phone language)", "$WHISPER/ggml-base-q5_1.bin", ModelFit.Model.WHISPER_BASE),
            Recommended("ggml-tiny.en-q5_1.bin", 32, "Fastest", "Basic quality", "English only", "$WHISPER/ggml-tiny.en-q5_1.bin", ModelFit.Model.WHISPER_TINY),
            Recommended("ggml-small.en-q5_1.bin", 190, "About 3× slower", "Better quality", "English only", "$WHISPER/ggml-small.en-q5_1.bin", ModelFit.Model.WHISPER_SMALL),
        )
        Screen.PARAKEET -> listOf(
            Recommended(
                "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", 661, "Fast", "Best quality",
                "English only; WER 1.2% vs 3.7% for whisper base. Unpack on a computer, copy the folder (encoder, decoder, joiner .onnx + tokens.txt)",
                "$SHERPA/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2", ModelFit.Model.PARAKEET,
            ),
        )
        Screen.HEBREW -> listOf(
            Recommended(
                "zipformer-he (ivrit-ai/zipformer2-66m-streaming-he)", 72, "Fastest", "Best Hebrew quality",
                "Hebrew only; 27 % word errors vs 51–67 % for whisper. Download encoder, decoder, joiner .int8.onnx + tokens.txt from sherpa-onnx/. CC-BY-4.0, ivrit.ai",
                ZIPFORMER_HE, ModelFit.Model.ZIPFORMER_HE,
            ),
            Recommended(
                "parakeet-v3-he (krut42/voice-parakeet-he-int8)", 673, "Fast", "Good Hebrew quality",
                "Hebrew only; 29 % word errors. Download the four files (encoder, decoder, joiner .int8.onnx + tokens.txt). CC-BY-4.0",
                PARAKEET_HE, ModelFit.Model.PARAKEET_HE,
            ),
        )
        Screen.LLM -> ModelFiles.LLM_RECOMMENDATIONS.mapIndexed { i, r ->
            Recommended(r.file, r.sizeMb, if (i == 0) "Fastest" else "About 3× slower", if (i == 0) "Good quality" else "Best quality", r.note, r.url, r.model)
        }
    }

    /** The screen's model the table recommends for [p] (null: none on this screen, e.g. whisper when Parakeet is the pick). */
    fun pick(s: Screen, p: ModelFit.Picks): ModelFit.Model? = when (s) {
        Screen.WHISPER -> p.speech.takeIf { it == ModelFit.Model.WHISPER_TINY || it == ModelFit.Model.WHISPER_BASE }
        Screen.PARAKEET -> p.speech.takeIf { it == ModelFit.Model.PARAKEET }
        Screen.HEBREW -> p.hebrew
        Screen.LLM -> p.cleanup
    }

    /**
     * Whether whisper base means the English-only file: the dictation language when it is a fixed
     * code, else (Automatic, Keyboard / Phone language) the phone's language [phone].
     */
    fun englishWhisper(language: String, phone: String): Boolean {
        val fixed = language.takeUnless { it == Settings.LANGUAGE_AUTO || it == Settings.LANGUAGE_KEYBOARD || it.isBlank() }
        return LanguageDefault.normalize(fixed ?: phone)?.substringBefore('-')?.substringBefore('_') == "en"
    }

    /** The Hebrew page's row for an imported model's kind. */
    fun hebrewModel(k: HebrewModels.Kind): ModelFit.Model = when (k) {
        HebrewModels.Kind.ZIPFORMER -> ModelFit.Model.ZIPFORMER_HE
        HebrewModels.Kind.PARAKEET -> ModelFit.Model.PARAKEET_HE
    }

    /**
     * Whether [r] is already on the phone (its download then needs no disk): whisper and AI
     * cleanup by file name ([names]); the Parakeet page's one model by any imported folder; the
     * Hebrew rows by the kind of each imported model ([hebrew]), since their folders keep no
     * recognisable name.
     */
    fun installed(s: Screen, r: Recommended, names: List<String>, hebrew: Set<ModelFit.Model> = emptySet()): Boolean = when (s) {
        Screen.HEBREW -> r.model in hebrew
        Screen.PARAKEET -> names.isNotEmpty()
        Screen.WHISPER, Screen.LLM -> r.file in names
    }

    /** What a recommended row says about this phone: the Recommended badge and its fits / doesn't-fit line (D117). */
    data class RowFit(val recommended: Boolean, val fits: Boolean, val line: String)

    /**
     * [english]: the whisper page badges only one of base.en / base, the one [englishWhisper]
     * picks.
     */
    fun rowFit(s: Screen, r: Recommended, m: ModelFit.Machine, installed: Boolean = false, english: Boolean = true): RowFit? {
        val model = r.model ?: return null
        val f = ModelFit.fit(model, r.sizeMb * 1_000_000L, installed, m)
        val base = if (english) "ggml-base.en-q5_1.bin" else "ggml-base-q5_1.bin"
        val picked = model == pick(s, ModelFit.recommend(m)) && (model != ModelFit.Model.WHISPER_BASE || r.file == base)
        return RowFit(picked, f.fits, ModelFit.line(f))
    }

    fun subtitle(r: Recommended) = "~${r.sizeMb} MB · ${r.speed} · ${r.quality} · ${r.note}"

    fun size(bytes: Long): String =
        if (bytes >= 1_000_000_000) String.format(Locale.ROOT, "%.1f GB", bytes / 1e9) else "${bytes / 1_000_000} MB"

    /** Status card "Active model" value. */
    fun active(name: String?): String = name ?: "None imported"

    fun engine(s: Screen) = when (s) {
        Screen.WHISPER -> "whisper.cpp (ggml)"
        Screen.PARAKEET -> "Parakeet-TDT via sherpa-onnx"
        Screen.LLM -> "llama.cpp (GGUF)"
        Screen.HEBREW -> "sherpa-onnx (zipformer or Parakeet-TDT)"
    }

    /** The line under RECOMMENDED: where the files come from. */
    fun recommendedNote(folder: Boolean, server: Boolean): String {
        val what = if (folder) "folder" else "file"
        return if (server) "Get these onto your server (its library folder) to download them here, or download on a computer, copy the $what to the phone and import it."
        else "Download on a computer (this app can't), copy the $what to the phone, then import it."
    }

    fun imported(bytes: Long, active: Boolean) = size(bytes) + if (active) " · In use" else ""

    fun importLabel(s: Screen, busy: Boolean) = when {
        busy -> "Importing…"
        s == Screen.PARAKEET || s == Screen.HEBREW -> "Import model folder…"
        else -> "Import model file…"
    }

    /** [server]: this build can also download from the set server's library (D41, hybrid / remote). */
    fun intro(s: Screen, server: Boolean = false) = when (s) {
        Screen.WHISPER -> if (server) "Copy a whisper.cpp model file (ggml-*.bin) to this phone and import it, or download one from your own server. " +
            "The app downloads only from your server; the links under Recommended are for a computer."
        else "Copy a whisper.cpp model file (ggml-*.bin) to this phone, then import it. Murmur never downloads anything."
        Screen.PARAKEET -> "English only and more accurate than whisper base. Copy the unpacked model folder to the phone, then pick that folder."
        Screen.HEBREW -> "Used only when the dictation language is Hebrew; the phone then loads this model instead of the English one " +
            "(switching language reloads, a few seconds). Put the four files in one folder (or one .zip), copy it to the phone and import it. " +
            "Files are checked against the pinned sizes and SHA-256. " +
            "Add ggml-tiny-q5_1.bin (32 MB, whisper.cpp) to the folder, or import it alone, and Automatic hears when you speak Hebrew " +
            "instead of following the keyboard."
        Screen.LLM -> "Optional. The model the \"This phone\" cleanup engine runs: a small language model that polishes each transcript " +
            "on the phone itself (punctuation, fillers, \"scratch that\" corrections, lists). " +
            (if (server) "Import a GGUF file you copied to the phone, or download one from your own server. "
            else "Import a GGUF file you copied to the phone (the links under Recommended are for a computer). ") +
            "Turn \"This phone\" on under Settings → AI cleanup → Engines to use it. Expect 1–3 s extra per dictation; " +
            "if cleanup takes longer than 3 s the formatted transcript is inserted instead."
    }
}
