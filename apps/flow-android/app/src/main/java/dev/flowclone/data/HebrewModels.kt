package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.LanguageModel
import dev.flowclone.core.ParakeetDecoding

/**
 * H5: the Hebrew model slot (docs/proposals/hebrew-stt.md, D33). The user imports one of the
 * pinned models (docs/findings/2026-10-30-hebrew-model-pins.md) as a folder or a .zip; the files
 * are checked against the pinned sizes and SHA-256 values, so only a known model is ever loaded.
 * Dictation in Hebrew then loads only that model (the phone keeps one ASR model in RAM,
 * decision 4); any other language loads the default engine again.
 */
object HebrewModels {
    /** The language key in `ModelConfig.languages`. */
    const val LANGUAGE = "he"

    data class Pin(val name: String, val size: Long, val sha256: String)

    /** The four files both pinned models consist of (same names; sizes tell them apart). */
    val FILES = listOf("encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt")

    enum class Kind(val engine: AsrEngine, val dirName: String, val label: String, val pins: List<Pin>) {
        /** ivrit-ai/zipformer2-66m-streaming-he int8: the default (26.8 % WER, 69 MiB). */
        ZIPFORMER(
            AsrEngine.SHERPA_ONLINE, "zipformer-he", "Zipformer Hebrew (ivrit.ai)",
            listOf(
                Pin("encoder.int8.onnx", 70478964, "6c6febcb74a09435315a1d872a6539f6e7fc2c333533f14c74edc0e85c49ffcd"),
                Pin("decoder.int8.onnx", 1049105, "feb03e7c8aa761593a9284a6fa1e91259be3702de75f630062256c1a59c5a906"),
                Pin("joiner.int8.onnx", 771805, "1d2c6d8e6dabc5eb52de52e95718e9bdfbffbfe7ead3808fb5ffd74e83bd9cae"),
                Pin("tokens.txt", 17086, "e4ebb5e72a6aeef47437a55fa2308ebbedb7c51c557de7a89bf965c23ea9af4a"),
            ),
        ),

        /** krut42/voice-parakeet-he-int8: the alternative (28.8 % WER, 642 MiB, ~1 GB RAM). */
        PARAKEET(
            AsrEngine.PARAKEET, "parakeet-v3-he", "Parakeet v3 Hebrew",
            listOf(
                Pin("encoder.int8.onnx", 653215173, "7eaf80bc88fccd579a7d3950c25e50e4c7935e5b037ec2aa1eee29bca5ca3981"),
                Pin("decoder.int8.onnx", 12494234, "3a1d0bbf3e393909095f826b7719fdaef42cd2e0b8b95c54518034ecc4b7d246"),
                Pin("joiner.int8.onnx", 7008293, "056c5464464b38a9cbacdccb6a9ff2114919780badb5c48e0f0260af7f91ccf2"),
                Pin("tokens.txt", 107590, "55a91c862d0a63e8b9587a3264ced4fc76943e66098468b4638e6a6c5642e364"),
            ),
        );

        val totalBytes: Long get() = pins.sumOf { it.size }
    }

    /** What is known about one file: [size] -1 = unknown, [sha256] null = not hashed (yet). */
    data class Facts(val size: Long, val sha256: String? = null)

    sealed interface Check {
        data class Ok(val kind: Kind) : Check
        data class Bad(val message: String) : Check
    }

    private fun fits(p: Pin, f: Facts) =
        (f.size < 0 || f.size == p.size) && (f.sha256 == null || f.sha256.equals(p.sha256, ignoreCase = true))

    private fun matches(kind: Kind, files: Map<String, Facts>) = kind.pins.all { p -> files[p.name]?.let { fits(p, it) } == true }

    /**
     * Which pinned model [files] (by base name) are. Extra files are ignored. With unknown sizes
     * and no hashes both models match and ZIPFORMER is returned: the copy re-checks with hashes.
     */
    fun identify(files: Map<String, Facts>): Check {
        val missing = FILES.filter { it !in files }
        if (missing.isNotEmpty()) return Check.Bad("Not a Hebrew model; missing ${missing.joinToString()}.")
        Kind.entries.firstOrNull { matches(it, files) }?.let { return Check.Ok(it) }
        // Name the first file that differs from the model its encoder size points at.
        val enc = files.getValue(FILES[0]).size
        val near = Kind.entries.minBy { k -> kotlin.math.abs(k.pins[0].size - enc) }
        val bad = near.pins.first { p -> !fits(p, files.getValue(p.name)) }
        val f = files.getValue(bad.name)
        val why = if (f.size >= 0 && f.size != bad.size) "is ${f.size} bytes, expected ${bad.size}" else "has a different SHA-256 (corrupt or another version)"
        return Check.Bad("Not a supported Hebrew model: ${bad.name} $why for ${near.label}. Use the pinned files linked below.")
    }

    /** The base name of a zip entry / document ("sherpa-onnx/encoder.int8.onnx" → "encoder.int8.onnx"). */
    fun baseName(path: String) = path.trimEnd('/').substringAfterLast('/').substringAfterLast('\\')

    /** The kind of an imported directory (sizes only: hashes were checked at import), or null. */
    fun kindOf(dir: java.io.File): Kind? {
        if (!dir.isDirectory) return null
        val files = FILES.mapNotNull { n -> java.io.File(dir, n).takeIf { it.isFile }?.let { n to Facts(it.length()) } }.toMap()
        return (identify(files) as? Check.Ok)?.kind
    }

    /** Parakeet-he's search: the Developer override, else greedy (D22; D68's dictionary beam is English only). */
    fun decoding(override: ParakeetDecoding?): ParakeetDecoding = override ?: ParakeetDecoding.GREEDY

    /** The `ModelConfig.languages` entry for an imported model. Parakeet-he decodes greedy on the phone (D22). */
    fun languageModel(dir: String, kind: Kind, parakeetDecoding: ParakeetDecoding = ParakeetDecoding.GREEDY) =
        languageModel(dir, Installed(kind.engine, kind.label, kind), parakeetDecoding)

    fun languageModel(dir: String, m: Installed, parakeetDecoding: ParakeetDecoding = ParakeetDecoding.GREEDY) =
        LanguageModel(m.engine, dir, if (m.engine == AsrEngine.PARAKEET) parakeetDecoding else ParakeetDecoding.AUTO)

    /** What the Hebrew slot holds: a pinned [kind], or (D41, [kind] null) a custom model from the server's library. */
    data class Installed(val engine: AsrEngine, val label: String, val kind: Kind? = null)

    /**
     * D41: a custom Hebrew model downloaded from the server carries this file (`{"engine":
     * "sherpa_online" | "parakeet", "title": …}`), written by the hybrid / remote installer after
     * every file verified against the server's SHA-256. Pinned models never need it.
     */
    const val ENGINE_FILE = "flow-model.json"

    /** The Hebrew model in [dir]: a pinned one by its sizes, else a custom one by its [ENGINE_FILE]; null = none / incomplete. */
    fun installed(dir: java.io.File): Installed? {
        kindOf(dir)?.let { return Installed(it.engine, it.label, it) }
        if (!ParakeetFiles.isModelDir(dir)) return null
        val o = runCatching { org.json.JSONObject(java.io.File(dir, ENGINE_FILE).readText()) }.getOrNull() ?: return null
        val engine = when (o.optString("engine")) {
            "sherpa_online" -> AsrEngine.SHERPA_ONLINE
            "parakeet" -> AsrEngine.PARAKEET
            else -> return null
        }
        return Installed(engine, o.optString("title").ifBlank { dir.name }, null)
    }

    /** True when [language] (a code like `he`, `he-IL`, legacy `iw`) is Hebrew. */
    fun isHebrew(language: String?): Boolean {
        val p = language?.trim()?.lowercase()?.substringBefore('-')?.substringBefore('_') ?: return false
        return p == "he" || p == "iw" || p == "hebrew"
    }

    /**
     * The language whose model the phone loads ([LANGUAGE] or null = the default engine): Hebrew
     * routes only when a Hebrew model is installed. Compare old and new to decide a reload.
     */
    fun route(effectiveLanguage: String?, installed: Boolean): String? =
        LANGUAGE.takeIf { installed && isHebrew(effectiveLanguage) }
}

/**
 * The speech models the phone keeps resident for the next dictation. The engine holder reloads
 * only when this changes: with [Both] a keyboard switch never reloads, and toggling "Keep both
 * languages loaded" reloads once.
 */
sealed interface LoadSet {
    /** The default engine alone (whisper / Parakeet). */
    data object Default : LoadSet
    /** One language's own model alone (`load_language`, D33 decision 4: one model in RAM). */
    data class Language(val code: String) : LoadSet
    /** D54: the default model and the Hebrew model together. */
    data object Both : LoadSet

    companion object {
        /**
         * [Both] when [keepBoth] is on and both a Hebrew model ([hebrewInstalled]) and a default
         * model are installed, else [HebrewModels.route]. [defaultInstalled] is only asked when
         * [keepBoth] is on (it touches the file system).
         */
        fun of(effectiveLanguage: String?, hebrewInstalled: Boolean, keepBoth: Boolean, defaultInstalled: () -> Boolean): LoadSet = when {
            keepBoth && hebrewInstalled && defaultInstalled() -> Both
            else -> HebrewModels.route(effectiveLanguage, hebrewInstalled)?.let(::Language) ?: Default
        }
    }
}

/**
 * H5 import: copies a Hebrew model's four files into a staging directory, hashing while it
 * copies, then [HebrewModels.identify] decides with sizes and SHA-256. JVM-testable (no Android).
 */
object HebrewImport {
    /** [onBytes] gets the running total (for the progress line). */
    fun copyHashed(input: java.io.InputStream, dest: java.io.File, onBytes: (Long) -> Unit = {}): HebrewModels.Facts {
        val md = java.security.MessageDigest.getInstance("SHA-256")
        var total = 0L
        dest.outputStream().use { out ->
            val buf = ByteArray(1 shl 20)
            while (true) {
                val n = input.read(buf); if (n < 0) break
                out.write(buf, 0, n); md.update(buf, 0, n); total += n
                onBytes(n.toLong())
            }
        }
        return HebrewModels.Facts(total, md.digest().joinToString("") { "%02x".format(it) })
    }

    /**
     * Extracts the model files of a .zip (any folder depth; other entries skipped) into [dir].
     * Entry names are reduced to [HebrewModels.FILES] names, so nothing lands outside [dir].
     */
    fun extractZip(input: java.io.InputStream, dir: java.io.File, onBytes: (Long) -> Unit = {}): Map<String, HebrewModels.Facts> {
        val out = HashMap<String, HebrewModels.Facts>()
        java.util.zip.ZipInputStream(input).use { z ->
            while (true) {
                val e = z.nextEntry ?: break
                val name = HebrewModels.baseName(e.name)
                if (e.isDirectory || name !in HebrewModels.FILES || name in out) continue
                out[name] = copyHashed(z, java.io.File(dir, name), onBytes)
            }
        }
        return out
    }
}
