package dev.murmur.data

/**
 * Which models suit this phone (D117): the Kotlin twin of murmur-core's `model_fit` (the
 * machine-class table and its numbers are documented there). `ModelFitVectorsTest` checks it
 * against `crates/murmur-core/fixtures/model-fit/vectors.json`, which the Rust side checks too.
 * The phone has no GPU build, so its machines always say "no GPU".
 */
object ModelFit {
    enum class Gpu { NO, LLM, WHISPER }

    enum class Model(val key: String, val ramGb: Int, val gpu: Gpu = Gpu.NO) {
        PARAKEET("parakeet", 4),
        WHISPER_TINY("whisper-tiny", 0),
        WHISPER_BASE("whisper-base", 0),
        WHISPER_SMALL("whisper-small", 0),
        WHISPER_TURBO("whisper-turbo", 0, Gpu.WHISPER),
        ZIPFORMER_HE("zipformer-he", 0),
        PARAKEET_HE("parakeet-he", 4),
        IVRIT_TURBO("ivrit-turbo", 0, Gpu.WHISPER),
        QWEN05("qwen2.5-0.5b", 0),
        QWEN15("qwen2.5-1.5b", 8),
        LLAMA3B("llama-3.2-3b", 8, Gpu.LLM);

        companion object {
            fun of(key: String): Model? = entries.firstOrNull { it.key == key }
        }
    }

    /** [totalRam] 0 = unknown; [freeDisk] null = unknown. */
    data class Machine(val totalRam: Long, val freeDisk: Long? = null, val gpuLlm: Boolean = false, val gpuWhisper: Boolean = false)

    data class Picks(val speech: Model, val cleanup: Model?, val hebrew: Model)

    data class Fit(val fits: Boolean, val reasons: List<String>)

    private const val GIB = 1L shl 30

    /** Room kept free next to a model (as the import checks). */
    const val DISK_MARGIN = 100_000_000L

    /** At least 85 % of [gb] GiB (the OS reports less than the nominal RAM); unknown RAM passes. */
    fun hasRam(totalRam: Long, gb: Int): Boolean = totalRam == 0L || gb == 0 || totalRam * 100 >= gb * GIB * 85

    fun recommend(m: Machine): Picks {
        val speech = when {
            hasRam(m.totalRam, 4) -> Model.PARAKEET
            hasRam(m.totalRam, 2) -> Model.WHISPER_BASE
            else -> Model.WHISPER_TINY
        }
        val cleanup = when {
            !hasRam(m.totalRam, 8) -> null
            m.gpuLlm -> Model.LLAMA3B
            else -> Model.QWEN15
        }
        return Picks(speech, cleanup, if (m.gpuWhisper) Model.IVRIT_TURBO else Model.ZIPFORMER_HE)
    }

    fun fit(model: Model, size: Long, installed: Boolean, m: Machine): Fit {
        val reasons = mutableListOf<String>()
        val free = m.freeDisk
        if (!installed && free != null && free < size + DISK_MARGIN) reasons += "needs ${bytesUp(size + DISK_MARGIN)} free"
        if (!hasRam(m.totalRam, model.ramGb)) reasons += "needs ${model.ramGb} GB RAM"
        val gpuOk = when (model.gpu) {
            Gpu.NO -> true
            Gpu.LLM -> m.gpuLlm
            Gpu.WHISPER -> m.gpuWhisper
        }
        if (!gpuOk) reasons += "needs a graphics chip"
        return Fit(reasons.isEmpty(), reasons)
    }

    /** Rounded up: "2.2 GB", "95 MB" (decimal units, as [ModelPages.size]). */
    fun bytesUp(b: Long): String = if (b >= 1_000_000_000) {
        val tenths = (b + 99_999_999) / 100_000_000
        "${tenths / 10}.${tenths % 10} GB"
    } else "${(b + 999_999) / 1_000_000} MB"

    /** The line under a recommended file: "Fits this phone" or "Doesn't fit: needs 8 GB RAM" (still importable). */
    fun line(f: Fit): String = if (f.fits) "Fits this phone" else "Doesn't fit: " + f.reasons.joinToString(", ")
}
