package dev.murmur.ui

import android.app.Activity

/**
 * Every screen is built with [DevUi] (the Settings look) since feat/screens-ui; the old
 * heading / body / button page helpers are gone. Only the dp conversion is still shared.
 */
internal fun Activity.dp(v: Int) = (v * resources.displayMetrics.density).toInt()
