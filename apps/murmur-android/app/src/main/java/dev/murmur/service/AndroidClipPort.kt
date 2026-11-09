package dev.murmur.service

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import android.os.SystemClock
import dev.murmur.insert.ClipboardGuard

/**
 * [ClipboardGuard.Port] over the system clipboard. Clips are marked sensitive
 * (`EXTRA_IS_SENSITIVE`, API 33+ = every supported release) so the clipboard preview and
 * keyboards' clipboard history mask them. Reading the clipboard can be refused in the
 * background (Android 10+): then the change listener decides whether the clip is still ours.
 */
class AndroidClipPort(context: Context) : ClipboardGuard.Port {
    private val cm = context.getSystemService(ClipboardManager::class.java)
    /** Change events our own set / restore / clear will cause (not someone else's copy). */
    private var ownEvents = 0
    @Volatile private var lastForeignChange = 0L

    init {
        cm?.addPrimaryClipChangedListener {
            synchronized(this) { if (ownEvents > 0) ownEvents-- else lastForeignChange = SystemClock.elapsedRealtime() }
        }
    }

    override fun current(): ClipboardGuard.Clip? = runCatching {
        val c = cm?.primaryClip ?: return null
        ClipboardGuard.Clip(c.description?.extras?.getString(EXTRA_MARKER), c)
    }.getOrNull()

    override fun setSensitive(text: String, marker: String) {
        val clip = ClipData.newPlainText(LABEL, text).apply {
            description.extras = PersistableBundle().apply {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
                putString(EXTRA_MARKER, marker)
            }
        }
        own { cm?.setPrimaryClip(clip) }
    }

    override fun restore(raw: Any) { (raw as? ClipData)?.let { own { cm?.setPrimaryClip(it) } } }

    override fun clear() = own { cm?.clearPrimaryClip() }

    override fun changedSince(since: Long) = lastForeignChange >= since

    override fun now() = SystemClock.elapsedRealtime()

    private fun own(f: () -> Unit) {
        synchronized(this) { ownEvents++ }
        runCatching(f).onFailure { synchronized(this) { ownEvents-- } }
    }

    companion object {
        const val LABEL = "Murmur"
        const val EXTRA_MARKER = "dev.murmur.clip_marker"
    }
}
