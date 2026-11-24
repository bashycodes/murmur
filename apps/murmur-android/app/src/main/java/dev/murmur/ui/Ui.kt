package dev.murmur.ui

import android.app.Activity
import android.content.Intent
import android.net.Uri

/**
 * Every screen is built with [DevUi] (the Settings look) since feat/screens-ui; the old
 * heading / body / button page helpers are gone. Only these small helpers are still shared.
 */
internal fun Activity.dp(v: Int) = (v * resources.displayMetrics.density).toInt()

/** Opens [url] in the browser; does nothing when no app can show it. */
internal fun Activity.openInBrowser(url: String) {
    runCatching { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}
