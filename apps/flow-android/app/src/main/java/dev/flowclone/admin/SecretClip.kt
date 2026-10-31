package dev.flowclone.admin

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle

/**
 * Copy a secret (a new server key) as a **sensitive** clip (`ClipDescription.EXTRA_IS_SENSITIVE`,
 * API 33+ = every supported release): the system clipboard preview and keyboards' clipboard
 * history mask it. The label names the key, never the secret.
 */
object SecretClip {
    fun clip(label: String, secret: String): ClipData = ClipData.newPlainText(label, secret).apply {
        description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
    }

    /** False when there is no clipboard service (or it refused). */
    fun copy(ctx: Context, label: String, secret: String): Boolean = runCatching {
        val cm = ctx.getSystemService(ClipboardManager::class.java) ?: return false
        cm.setPrimaryClip(clip(label, secret))
        true
    }.getOrDefault(false)
}
