package dev.flowclone.data

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.debug.InsertOutcome
import dev.flowclone.debug.Wav
import java.io.File

/**
 * D35 "Retry with current settings": the recordings of the newest History rows, as 16 kHz mono
 * WAV files in app-private storage (`files/retry_audio/<row id>.wav`). Files, not SQLite blobs:
 * a 5-minute recording is ~9.6 MB and Android's cursor window holds ~2 MB per row. Deleting a
 * file is all there is to forgetting a recording.
 */
class RetryAudioStore(private val dir: File) {
    private fun file(id: Long) = File(dir, "$id$EXT")

    @Synchronized
    fun save(id: Long, pcm: ShortArray) {
        if (pcm.isEmpty()) return
        dir.mkdirs()
        val tmp = File(dir, "$id$EXT.tmp")
        tmp.writeBytes(Wav.encode(pcm))
        if (!tmp.renameTo(file(id))) { tmp.delete(); error("couldn't save the recording") }
    }

    fun has(id: Long): Boolean = file(id).isFile

    /** The recording of row [id], or null when it is gone (or unreadable). */
    fun load(id: Long): ShortArray? = runCatching { Wav.decode(file(id).readBytes()) }.getOrNull()

    /** Ids that have a recording. */
    @Synchronized
    fun ids(): Set<Long> =
        dir.listFiles().orEmpty().mapNotNull { f -> f.name.takeIf { it.endsWith(EXT) }?.removeSuffix(EXT)?.toLongOrNull() }.toSet()

    @Synchronized
    fun delete(id: Long) { file(id).delete() }

    /** Keep only the recordings of [keep]; every other file (leftover temp files too) is deleted. */
    @Synchronized
    fun retain(keep: Set<Long>) {
        for (f in dir.listFiles().orEmpty()) {
            val id = f.name.takeIf { it.endsWith(EXT) }?.removeSuffix(EXT)?.toLongOrNull()
            if (id == null || id !in keep) f.delete()
        }
    }

    @Synchronized
    fun clear() = retain(emptySet())

    private companion object { const val EXT = ".wav" }
}

/** One text of a retried History row: 0 = the original dictation, then one per retry. */
data class HistoryVersion(
    val n: Int,
    val createdAt: Long,
    val status: String,
    val asrText: String,
    val finalText: String,
    val route: String? = null,
) {
    /** What the card shows for this version (the cleaned text, else the raw transcript). */
    val text: String get() = finalText.ifBlank { asrText }
}

/** Pure rules of "Retry with current settings" (D35); the stores and the service apply them. */
object RetryLogic {
    /** "Keep recordings for retry" default: the newest 20 History rows. */
    const val DEFAULT_KEEP = 20
    const val MAX_KEEP = 200
    /** The History page's choices (0 = off). */
    val KEEP_CHOICES = listOf(0, 5, 10, 20, 50, 100)

    const val RETRY_LABEL = "Retry with current settings"
    const val GONE = "The recording is no longer kept"
    const val BUSY = "Finish the current dictation first"
    const val RUNNING = "A retry is already running"
    const val EMPTY = "No speech found in the recording"
    const val DONE = "Retried: the newest version is shown"

    fun keepOf(n: Int): Int = n.coerceIn(0, MAX_KEEP)

    fun keepLabel(n: Int): String = when (n) {
        0 -> "Off"
        1 -> "Newest dictation"
        else -> "Newest $n dictations"
    }

    /** The row ids whose recording stays: the newest [keep] rows ([newestFirst] = every row, newest first). */
    fun keepIds(newestFirst: List<Long>, keep: Int): Set<Long> = newestFirst.take(keepOf(keep)).toSet()

    /**
     * Keep this dictation's recording? Never for Command Mode, never when nothing is saved, never
     * from a password / sensitive field, a banking or denylisted app, or a protected target.
     */
    fun keepsRecording(
        keep: Int, retention: Retention, command: Boolean, outcome: String?,
        protectedField: Boolean, protectedApp: Boolean,
    ): Boolean = keep > 0 && retention.stores && !command && !protectedField && !protectedApp &&
        outcome != InsertOutcome.TARGET_PROTECTED

    /** Card ⋮ menu: Retry only while the recording is kept (never for a command row). */
    enum class MenuItem(val label: String) { DETAILS("Open details"), RETRY(RETRY_LABEL), DELETE("Delete") }

    fun menu(row: HistoryRow, hasAudio: Boolean): List<MenuItem> =
        listOfNotNull(MenuItem.DETAILS, MenuItem.RETRY.takeIf { canRetry(row, hasAudio) }, MenuItem.DELETE)

    fun canRetry(row: HistoryRow, hasAudio: Boolean) = hasAudio && !row.isCommand

    /** Why a retry gave no new version, or null when [r] is one. */
    fun failure(r: CommitResult): String? = when {
        r.status == CommitStatus.ERROR -> r.error?.trim()?.ifEmpty { null } ?: "Transcription failed"
        r.status == CommitStatus.EMPTY || r.historyText.isBlank() -> EMPTY
        else -> null
    }

    /**
     * [old] after a retry produced [r]: the new texts, route and timings; when, where, how it was
     * inserted and the recording length stay the original's.
     */
    fun retried(old: HistoryRow, r: CommitResult): HistoryRow = old.copy(
        status = r.status.name, asrText = r.asrText, finalText = r.historyText,
        fallbackReason = r.fallbackReason, route = r.route?.engine,
        asrMs = r.timings.asrMs, llmMs = r.timings.llmMs, totalMs = r.timings.totalMs,
    )

    /** Version 0 of a row retried for the first time: its text as it was. */
    fun original(row: HistoryRow) = HistoryVersion(0, row.createdAt, row.status, row.asrText, row.finalText, row.route)

    /** Words Insights counted for a row's text (failed rows were never counted). */
    fun countedWords(status: String, text: String): Long =
        if (status == CommitStatus.ERROR.name) 0 else Insights.words(text).toLong()

    /** Pill labels: "0", "1", … (nothing for a row never retried). */
    fun pills(versions: List<HistoryVersion>): List<String> =
        if (versions.size < 2) emptyList() else versions.map { it.n.toString() }
}
