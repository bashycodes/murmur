package dev.flowclone.notes

import org.json.JSONObject
import java.io.File
import kotlin.random.Random

/** One saved note as the Notes list shows it. Times are epoch ms. */
data class Note(val id: String, val text: String, val pinned: Boolean, val createdAt: Long, val updatedAt: Long)

/**
 * Notes: the Scratchpad's `files/notes/<id>.txt` files, plus `index.json` next to them holding
 * what a text file cannot (pinned, created, updated). The text files stay the source of truth:
 *  - a file without an index entry is a note (created = updated = its modified time); that is
 *    how the Scratchpad from before Notes shows up, pinned unless the user unpinned it
 *  - an index entry without a file is ignored and dropped on the next index write
 *  - a broken index reads as empty (only pins and times are lost, never text)
 * Ids are [DocumentStore.isValidId] (checked on every call), so nothing leaves [dir].
 * Every call holds the store lock; the app also runs them all on [dev.flowclone.FlowApp.documentsIo].
 */
class NotesStore(private val dir: File, private val clock: () -> Long = System::currentTimeMillis) : DocumentStore {
    private val files = FileDocumentStore(dir)
    private val indexFile = File(dir, INDEX)

    private data class Meta(val pinned: Boolean, val created: Long, val updated: Long)

    @Synchronized override fun load(id: String): String? = files.load(id)

    /** Writes the text, then stamps the index (a failed index write leaves the text saved). */
    @Synchronized override fun save(id: String, text: String) {
        files.save(id, text)
        val now = clock()
        runCatching {
            val idx = existing(readIndex())
            val old = idx[id]
            idx[id] = Meta(old?.pinned ?: defaultPinned(id), old?.created ?: now, now)
            writeIndex(idx)
        }
    }

    /** Every note on disk: pinned first, then most recently updated. */
    @Synchronized fun list(): List<Note> {
        val idx = readIndex()
        return (dir.listFiles() ?: emptyArray())
            .filter { it.isFile && it.name.endsWith(TXT) }
            .mapNotNull { f ->
                val id = f.name.removeSuffix(TXT).takeIf(DocumentStore::isValidId) ?: return@mapNotNull null
                val text = runCatching { f.readText(Charsets.UTF_8) }.getOrNull() ?: return@mapNotNull null
                val m = idx[id]
                val mtime = f.lastModified()
                Note(id, text, m?.pinned ?: defaultPinned(id), m?.created ?: mtime, m?.updated ?: mtime)
            }
            .sortedWith(compareByDescending<Note> { it.pinned }.thenByDescending { it.updatedAt }.thenBy { it.id })
    }

    @Synchronized fun setPinned(id: String, pinned: Boolean) {
        require(DocumentStore.isValidId(id)) { "bad document id" }
        val idx = existing(readIndex())
        val f = File(dir, id + TXT)
        val mtime = if (f.isFile) f.lastModified() else clock()
        val old = idx[id] ?: Meta(defaultPinned(id), mtime, mtime)
        idx[id] = old.copy(pinned = pinned)
        writeIndex(idx)
    }

    /** Removes the note's text file and its index entry. */
    @Synchronized fun delete(id: String) {
        files.delete(id)
        val idx = existing(readIndex())
        idx.remove(id)
        runCatching { writeIndex(idx) }
    }

    /** A fresh id with no file yet; nothing is written until the editor saves text under it. */
    @Synchronized fun newId(): String {
        while (true) {
            val id = "n" + clock().toString(36) + "-" + Random.nextInt(36 * 36 * 36 * 36).toString(36).padStart(4, '0')
            if (DocumentStore.isValidId(id) && !File(dir, id + TXT).exists()) return id
        }
    }

    private fun defaultPinned(id: String) = id == DocumentStore.SCRATCHPAD

    /** Only entries whose file still exists (stale ones are dropped on write). */
    private fun existing(idx: Map<String, Meta>): MutableMap<String, Meta> =
        idx.filterKeys { File(dir, it + TXT).isFile }.toMutableMap()

    private fun readIndex(): Map<String, Meta> {
        if (!indexFile.isFile) return emptyMap()
        return runCatching {
            val notes = JSONObject(indexFile.readText(Charsets.UTF_8)).optJSONObject("notes") ?: return emptyMap()
            buildMap {
                for (id in notes.keys()) {
                    if (!DocumentStore.isValidId(id)) continue
                    val o = notes.optJSONObject(id) ?: continue
                    put(id, Meta(o.optBoolean("pinned", false), o.optLong("created", 0), o.optLong("updated", 0)))
                }
            }
        }.getOrDefault(emptyMap())
    }

    private fun writeIndex(idx: Map<String, Meta>) {
        val notes = JSONObject()
        for ((id, m) in idx.toSortedMap()) {
            notes.put(id, JSONObject().put("pinned", m.pinned).put("created", m.created).put("updated", m.updated))
        }
        dir.mkdirs()
        val tmp = File(dir, "$INDEX.tmp")
        tmp.writeText(JSONObject().put("version", 1).put("notes", notes).toString(), Charsets.UTF_8)
        if (!tmp.renameTo(indexFile)) { tmp.delete(); throw java.io.IOException("could not replace $INDEX") }
    }

    companion object {
        const val INDEX = "index.json"
        private const val TXT = ".txt"
    }
}

/** List titles, previews and search for the Notes screen. */
object NotesView {
    /** The Scratchpad's name in the list (and its editor). */
    const val QUICK_NOTE = "Quick note"
    const val EMPTY_TITLE = "Empty note"
    const val TITLE_MAX = 80
    private const val PREVIEW_MAX = 120

    private fun lines(n: Note) = n.text.lines().map { it.trim() }.filter { it.isNotEmpty() }

    private fun cut(s: String, max: Int) = if (s.length > max) s.take(max) + "…" else s

    fun title(n: Note): String = when {
        n.id == DocumentStore.SCRATCHPAD -> QUICK_NOTE
        else -> lines(n).firstOrNull()?.let { cut(it, TITLE_MAX) } ?: EMPTY_TITLE
    }

    /** The text after the title line, on one line (the Quick note's starts at its first line). */
    fun preview(n: Note): String {
        val l = lines(n)
        val rest = if (n.id == DocumentStore.SCRATCHPAD) l else l.drop(1)
        return cut(rest.joinToString(" "), PREVIEW_MAX)
    }

    /** Notes whose text (or title) contains [query], ignoring case; blank = all, order kept. */
    fun filter(notes: List<Note>, query: String): List<Note> {
        val q = query.trim()
        if (q.isEmpty()) return notes
        return notes.filter { it.text.contains(q, ignoreCase = true) || title(it).contains(q, ignoreCase = true) }
    }
}
