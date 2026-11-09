package dev.murmur.notes

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File

/**
 * Plain-text documents the user dictates into (the Scratchpad / Quick note and every note).
 * Platform-free and small on purpose: the editor ([dev.murmur.ui.NoteEditorActivity]) only
 * needs load + save by id; listing, pins and delete live in [NotesStore].
 */
interface DocumentStore {
    /** The saved text, or null when nothing was ever saved under [id]. */
    fun load(id: String): String?

    /** Replace [id]'s text (empty text is kept as an empty document). */
    fun save(id: String, text: String)

    companion object {
        /** The one Scratchpad document. */
        const val SCRATCHPAD = "scratchpad"

        private val ID = Regex("[a-z0-9][a-z0-9_-]{0,63}")

        /** Lower-case file-name-safe ids only, so an id can never leave the store's directory. */
        fun isValidId(id: String) = ID.matches(id)
    }
}

/**
 * One UTF-8 `<id>.txt` per document in [dir] (app-private storage, never backed up:
 * `allowBackup=false`). A save writes a temp file and renames it over the old one, so a kill
 * mid-write leaves the previous version, never half a note.
 */
class FileDocumentStore(private val dir: File) : DocumentStore {
    private fun file(id: String): File {
        require(DocumentStore.isValidId(id)) { "bad document id" }
        return File(dir, "$id.txt")
    }

    override fun load(id: String): String? {
        val f = file(id)
        return if (f.isFile) f.readText(Charsets.UTF_8) else null
    }

    @Synchronized
    override fun save(id: String, text: String) {
        val f = file(id)
        dir.mkdirs()
        val tmp = File(dir, "$id.txt.tmp")
        tmp.writeText(text, Charsets.UTF_8)
        if (!tmp.renameTo(f)) {
            tmp.delete()
            throw java.io.IOException("could not replace ${f.name}")
        }
    }

    /** Removes [id]'s file (already missing = done). */
    @Synchronized
    fun delete(id: String) {
        val f = file(id)
        if (f.exists() && !f.delete()) throw java.io.IOException("could not delete ${f.name}")
    }
}

/**
 * Debounced autosave for one open document. [edited] on every change; the text is written
 * [delayMs] after the edits pause (dictation inserts arrive as one change, typing as many).
 * [flush] writes now (the editor calls it on pause, so leaving or killing the app keeps the
 * text). Writes run in [scope] (an IO scope in the app), one at a time, and skip text equal to
 * the last saved version. A failed write keeps the text pending for the next save.
 */
class Autosaver(
    private val scope: CoroutineScope,
    initial: String,
    private val delayMs: Long = DEFAULT_DELAY_MS,
    private val write: (String) -> Unit,
) {
    private val lock = Mutex()
    @Volatile private var latest: String = initial
    /** Last text known to be on disk; only touched under [lock]. */
    private var saved: String = initial
    private var timer: Job? = null

    /** A debounced write is scheduled and has not run yet. */
    val pending: Boolean get() = timer?.isActive == true

    fun edited(text: String) {
        latest = text
        timer?.cancel()
        timer = scope.launch { delay(delayMs); save() }
    }

    fun flush() {
        timer?.cancel()
        timer = null
        scope.launch { save() }
    }

    private suspend fun save() = lock.withLock {
        val t = latest
        if (t == saved) return@withLock
        if (runCatching { write(t) }.isSuccess) saved = t
    }

    companion object {
        const val DEFAULT_DELAY_MS = 800L
    }
}

/** Word count for the editor's status line. */
object TextStats {
    /** Whitespace-separated tokens that contain a letter or digit (a lone "-" is not a word). */
    fun words(text: String): Int = text.split(Regex("\\s+")).count { t -> t.any { it.isLetterOrDigit() } }

    fun label(text: String): String = words(text).let { if (it == 1) "1 word" else "$it words" }
}
