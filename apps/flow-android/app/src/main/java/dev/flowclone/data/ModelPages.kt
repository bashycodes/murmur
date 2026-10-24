package dev.flowclone.data

import java.util.Locale

/**
 * Model import screens (ui/ModelImportActivity) in the Settings look: status card (active model,
 * size, engine), Import as the primary action, recommended files with their download links (the
 * app never downloads: D9), and the imported list. The import itself stays in [ModelFiles].
 */
object ModelPages {
    enum class Screen { WHISPER, PARAKEET, LLM }

    data class Recommended(val file: String, val sizeMb: Int, val speed: String, val quality: String, val note: String, val url: String)

    private const val WHISPER = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
    private const val SHERPA = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"

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
    }

    fun imported(bytes: Long, active: Boolean) = size(bytes) + if (active) " · In use" else ""

    fun importLabel(s: Screen, busy: Boolean) = when {
        busy -> "Importing…"
        s == Screen.PARAKEET -> "Import model folder…"
        else -> "Import model file…"
    }

    fun intro(s: Screen) = when (s) {
        Screen.WHISPER -> "Copy a whisper.cpp model file (ggml-*.bin) to this phone, then import it. Flow Clone never downloads anything."
        Screen.PARAKEET -> "English only and more accurate than whisper base. Copy the unpacked model folder to the phone, then pick that folder. Recommended on phones with 6–8 GB RAM."
        Screen.LLM -> "Optional. A small local language model polishes each transcript (punctuation, fillers, \"scratch that\" corrections, lists). " +
            "It runs only on this phone and is off until you turn it on in Settings → AI cleanup. Expect 1–3 s extra per dictation; " +
            "if cleanup takes longer than 3 s the formatted transcript is inserted instead."
    }
}
