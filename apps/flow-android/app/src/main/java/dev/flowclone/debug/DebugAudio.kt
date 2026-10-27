package dev.flowclone.debug

import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** 16 kHz mono PCM16 → a canonical 44-byte-header WAV. */
object Wav {
    fun encode(pcm: ShortArray, samples: Int = pcm.size, rate: Int = 16_000): ByteArray {
        val n = samples.coerceIn(0, pcm.size)
        val data = n * 2
        val b = ByteBuffer.allocate(44 + data).order(ByteOrder.LITTLE_ENDIAN)
        b.put("RIFF".toByteArray(Charsets.US_ASCII)).putInt(36 + data).put("WAVE".toByteArray(Charsets.US_ASCII))
        b.put("fmt ".toByteArray(Charsets.US_ASCII)).putInt(16).putShort(1).putShort(1).putInt(rate).putInt(rate * 2)
            .putShort(2).putShort(16)
        b.put("data".toByteArray(Charsets.US_ASCII)).putInt(data)
        for (i in 0 until n) b.putShort(pcm[i])
        return b.array()
    }

    /**
     * Debug "WAV as microphone": the samples of a 16 kHz mono PCM16 WAV (any chunk order; extra
     * chunks such as LIST are skipped). Anything else throws [IllegalArgumentException] saying why,
     * because the engine would hear it at the wrong speed or as noise.
     */
    fun decode(bytes: ByteArray, rate: Int = 16_000): ShortArray {
        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        fun tag(at: Int) = String(bytes, at, 4, Charsets.US_ASCII)
        require(bytes.size >= 12 && tag(0) == "RIFF" && tag(8) == "WAVE") { "not a RIFF/WAVE file" }
        var fmtOk = false
        var p = 12
        while (p + 8 <= bytes.size) {
            val id = tag(p)
            val size = b.getInt(p + 4).toLong() and 0xffffffffL
            val body = p + 8
            require(body + size <= bytes.size || id == "data") { "chunk '$id' runs past the end of the file" }
            when (id) {
                "fmt " -> {
                    require(size >= 16) { "fmt chunk too short" }
                    val format = b.getShort(body).toInt() and 0xffff
                    val channels = b.getShort(body + 2).toInt()
                    val sampleRate = b.getInt(body + 4)
                    val bits = b.getShort(body + 14).toInt()
                    require(format == 1) { "not PCM (format $format)" }
                    require(channels == 1) { "$channels channels; needs mono" }
                    require(sampleRate == rate) { "$sampleRate Hz; needs $rate Hz" }
                    require(bits == 16) { "$bits-bit; needs 16-bit" }
                    fmtOk = true
                }
                "data" -> {
                    require(fmtOk) { "no fmt chunk before the data" }
                    val n = (minOf(size, (bytes.size - body).toLong()) / 2).toInt()
                    return ShortArray(n) { b.getShort(body + 2 * it) }
                }
            }
            p = (body + size + (size and 1)).toInt()
        }
        throw IllegalArgumentException(if (fmtOk) "no data chunk" else "no fmt chunk")
    }
}

/**
 * Developer "Keep last dictation audio (debug)", default off: what the mic delivered for each of
 * the last [maxFiles] dictations, as WAV in app-private storage (`files/debug/audio/`), named by
 * the D21 line's `ts` (colons → dashes) and linked from its `audio` field, so a bad line can be
 * replayed on the host (flow-cli). Files older than [maxAgeMs] are deleted on every save and at
 * app start. Never written for protected targets ([DebugRedact.keepsAudio]); never uploaded.
 */
class DebugAudioStore(
    dir: File,
    private val maxFiles: Int = MAX_FILES,
    private val maxAgeMs: Long = MAX_AGE_MS,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    companion object {
        const val MAX_FILES = 30
        const val MAX_AGE_MS = 72 * 3_600_000L
        fun inFilesDir(filesDir: File) = DebugAudioStore(File(File(filesDir, "debug"), "audio"))
        fun nameFor(tsMs: Long): String = DebugFormat.ts(tsMs).replace(':', '-') + ".wav"
    }

    val dir: File = dir

    private fun wavs(): List<File> =
        dir.listFiles { f -> f.isFile && f.name.endsWith(".wav") }?.sortedByDescending { it.name }.orEmpty()

    /** Writes the WAV, then prunes. Returns the file name (the `audio` field). */
    @Synchronized fun save(tsMs: Long, pcm: ShortArray, samples: Int = pcm.size): String {
        dir.mkdirs()
        val name = nameFor(tsMs)
        val tmp = File(dir, "$name.tmp")
        tmp.writeBytes(Wav.encode(pcm, samples))
        val out = File(dir, name)
        if (!tmp.renameTo(out)) { out.writeBytes(tmp.readBytes()); tmp.delete() }
        out.setLastModified(clock())
        prune()
        return name
    }

    /** Newest [maxFiles] kept; anything older than [maxAgeMs] (by modification time) deleted. */
    @Synchronized fun prune() {
        val cutoff = clock() - maxAgeMs
        wavs().forEachIndexed { i, f -> if (i >= maxFiles || f.lastModified() < cutoff) f.delete() }
    }

    /** File names, newest first. */
    @Synchronized fun list(): List<String> = wavs().map { it.name }

    @Synchronized fun clear() { wavs().forEach { it.delete() } }
}
