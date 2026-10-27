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
