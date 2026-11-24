package dev.murmur.ui

import android.app.UiModeManager
import android.content.Context
import dev.murmur.data.AppTheme

/**
 * Applies Settings → Theme to the whole app: the per-app night mode switches the
 * app's configuration, so `values-night` and [Tokens.of] follow it, and open screens are
 * recreated by the system. System = follow the phone.
 */
object ThemeMode {
    fun nightMode(t: AppTheme): Int = when (t) {
        AppTheme.SYSTEM -> UiModeManager.MODE_NIGHT_AUTO
        AppTheme.LIGHT -> UiModeManager.MODE_NIGHT_NO
        AppTheme.DARK -> UiModeManager.MODE_NIGHT_YES
    }

    fun apply(ctx: Context, t: AppTheme) {
        ctx.getSystemService(UiModeManager::class.java)?.setApplicationNightMode(nightMode(t))
    }
}
