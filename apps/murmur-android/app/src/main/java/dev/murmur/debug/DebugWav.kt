package dev.murmur.debug

import dev.murmur.session.DictationState
import dev.murmur.session.InsertTarget
import java.io.File

/**
 * Debug "WAV as microphone" (debug builds only: the receiver lives in src/debug and MurmurService
 * acts on [ACTION] only when BuildConfig.DEBUG). The host pushes a 16 kHz mono WAV into
 * `files/debug/` and broadcasts to `.debug.WavMicReceiver` (tools/phone/play-wav.sh); one
 * normal dictation then runs on that audio instead of the mic.
 */
object DebugWav {
    /** MurmurService action (not exported: only the in-app receiver sends it). */
    const val ACTION = "dev.murmur.action.DEBUG_WAV"
    /** Broadcast action of the debug receiver. */
    const val BROADCAST = "dev.murmur.debug.PLAY_WAV"
    /** Path of the WAV, relative to the app's data dir (`files/debug/…`) or to `files/`. */
    const val EXTRA_PATH = "path"
    /** true = feed as fast as the pipeline takes it, not in real time. */
    const val EXTRA_FAST = "fast"
    /** true = a hands-free (tap) dictation (ends on silence or at the end of the WAV); default hold. */
    const val EXTRA_TAP = "tap"

    /** The WAV under `files/debug/`, or null (outside it, missing, not a file). */
    fun resolve(filesDir: File, path: String): File? {
        if (path.isBlank() || path.startsWith("/")) return null
        val rel = path.removePrefix("files/")
        val root = File(filesDir, "debug").canonicalFile
        val f = File(filesDir, rel).canonicalFile
        if (!f.path.startsWith(root.path + File.separator) || !f.isFile) return null
        return f
    }

    /**
     * Why a WAV dictation must not start now, or null. Busy: one dictation at a time. A protected
     * field (password, banking, denylisted app) is excluded up front, as the bubble is; the insert
     * guard would refuse it anyway.
     */
    fun refusal(state: DictationState, target: InsertTarget?): String? = when {
        state is DictationState.Recording || state is DictationState.Processing -> "busy: a dictation is running"
        target == InsertTarget.PROTECTED -> "protected target"
        else -> null
    }
}
