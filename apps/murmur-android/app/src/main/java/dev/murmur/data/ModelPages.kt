package dev.murmur.data

import java.util.Locale

/**
 * Model import screens (ui/ModelImportActivity) in the Settings look: status card (active model,
 * size, engine), Import as the primary action, recommended files with their download links (the
 * app never downloads: D9), and the imported list. The import itself stays in [ModelFiles].
 */
object ModelPages {
    enum class Screen { WHISPER, PARAKEET, LLM, HEBREW }

    data class Recommended(val file: String, val sizeMb: Int, val speed: String, val quality: String, val note: String, val url: String)

    private const val WHISPER = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
    private const val SHERPA = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
    /** H5: pinned revisions (docs/findings/2026-10-30-hebrew-model-pins.md). */
    const val ZIPFORMER_HE = "https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he/tree/ee769a2a13b887f91572eec349458bfb0fe79e7a/sherpa-onnx"
    const val PARAKEET_HE = "https://huggingface.co/krut42/voice-parakeet-he-int8/tree/39de3b473bed626c7796c156d71adc66161ebfe1"

    fun recommended(s: Screen): List<Recommended> = when (s) {
        Screen.WHISPER -> listOf(
            Recommended("ggml-base.en-q5_1.bin", 60, "Fast", "Good quality", "English only", "$WHISPER/ggml-base.en-q5_1.bin"),
            Recommended("ggml-base-q5_1.bin", 60, "Fast", "Good quality", "Many languages (follows the phone language)", "$WHISPER/ggml-base-q5_1.bin"),
            Recommended("ggml-tiny.en-q5_1.bin", 32, "Fastest", "Basic quality", "English only; older phones", "$WHISPER/ggml-tiny.en-q5_1.bin"),
            Recommended("ggml-small.en-q5_1.bin", 190, "About 3× slower", "Better quality", "English only; flagship phones", "$WHISPER/ggml-small.en-q5_1.bin"),
        )
        Screen.PARAKEET -> listOf(
            Recommended(
                "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", 661, "Fast", "Best quality",
                "English only; WER 1.2% vs 3.7% for whisper base. Unpack on a computer, copy the folder (encoder, decoder, joiner .onnx + tokens.txt). Needs ~1 GB free RAM",
                "$SHERPA/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
            ),
        )
        Screen.HEBREW -> listOf(
            Recommended(
                "zipformer-he (ivrit-ai/zipformer2-66m-streaming-he)", 72, "Fastest", "Best Hebrew quality",
                "Hebrew only; 27 % word errors vs 51–67 % for whisper. Download encoder, decoder, joiner .int8.onnx + tokens.txt from sherpa-onnx/. CC-BY-4.0, ivrit.ai",
                ZIPFORMER_HE,
            ),
            Recommended(
                "parakeet-v3-he (krut42/voice-parakeet-he-int8)", 673, "Fast", "Good Hebrew quality",
                "Hebrew only; 29 % word errors; needs ~1 GB free RAM. Download the four files (encoder, decoder, joiner .int8.onnx + tokens.txt). CC-BY-4.0",
                PARAKEET_HE,
            ),
        )
        Screen.LLM -> ModelFiles.LLM_RECOMMENDATIONS.mapIndexed { i, r ->
            Recommended(r.file, r.sizeMb, if (i == 0) "Fastest" else "About 3× slower", if (i == 0) "Good quality" else "Best quality", r.note, r.url)
        }
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
        Screen.WHISPER -> if (server) "Copy a whisper.cpp model file (ggml-*.bin) to this phone and import it, or download one from your own server. Nothing ever comes from the internet."
        else "Copy a whisper.cpp model file (ggml-*.bin) to this phone, then import it. Murmur never downloads anything."
        Screen.PARAKEET -> "English only and more accurate than whisper base. Copy the unpacked model folder to the phone, then pick that folder. Recommended on phones with 6–8 GB RAM."
        Screen.HEBREW -> "Used only when the dictation language is Hebrew; the phone then loads this model instead of the English one " +
            "(switching language reloads, a few seconds). Put the four files in one folder (or one .zip), copy it to the phone and import it. " +
            "Files are checked against the pinned sizes and SHA-256."
        Screen.LLM -> "Optional. A small local language model polishes each transcript (punctuation, fillers, \"scratch that\" corrections, lists). " +
            "It runs only on this phone and is off until you turn it on in Settings → AI cleanup. Expect 1–3 s extra per dictation; " +
            "if cleanup takes longer than 3 s the formatted transcript is inserted instead."
    }
}
