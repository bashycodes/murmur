package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D104: the theme default moved System → Dark; only a fresh install gets Dark ([ThemeDefault]). */
class ThemeDefaultTest {
    private fun MemKV.theme(): AppTheme = AppTheme.fromKey(getString(ThemeDefault.key))

    /** As `SettingsStore`: one emptiness check, then every keep step. */
    private fun MemKV.keepOld() = keepOldDefaults(this, empty = m.isEmpty())

    @Test fun freshEmptyStoreIsDark() {
        val kv = MemKV()
        kv.keepOld()
        assertNull("nothing written for the theme", kv.getString(ThemeDefault.key))
        assertTrue(kv.getBoolean(ThemeDefault.flag, false))
        assertEquals(AppTheme.DARK, kv.theme())
        assertEquals(AppTheme.DARK, Settings().theme)
    }

    @Test fun anUpgradeKeepsFollowingThePhone() {
        // Any install from before has written something (here only the language migration).
        val kv = MemKV().apply { m["language_default_device"] = true; m["language"] = "keyboard" }
        kv.keepOld()
        assertEquals("system", kv.getString(ThemeDefault.key))
        assertEquals(AppTheme.SYSTEM, kv.theme())
    }

    @Test fun anUpgradeThatAlreadyRanTheStrengthStepStillKeepsSystem() {
        // The D103 step set its flag on an earlier start; the store is not empty.
        val kv = MemKV().apply { m[StrengthDefault.MIGRATED] = true }
        kv.keepOld()
        assertEquals(AppTheme.SYSTEM, kv.theme())
    }

    @Test fun aStoredThemeIsNeverTouched() {
        for (t in AppTheme.entries) {
            val kv = MemKV().apply { m["language"] = "en"; m[ThemeDefault.key] = t.key }
            kv.keepOld()
            assertEquals(t, kv.theme())
        }
    }

    @Test fun theFlagIsSetOnceAndLaterSavesAreKept() {
        val kv = MemKV().apply { m["language"] = "en" }
        kv.keepOld()
        kv.m.remove(ThemeDefault.key)
        kv.keepOld()
        assertNull("not written again after the flag", kv.getString(ThemeDefault.key))
    }

    @Test fun restoringAnOldBackupKeepsSystem() {
        val kv = MemKV()
        kv.keepOld()
        // BackupIo.replacePrefs: clear, then write an older backup's keys (no flag, no theme).
        kv.edit { clear(); putString("language", "en") }
        kv.keepOld() // SettingsStore.load after the restore
        assertEquals(AppTheme.SYSTEM, kv.theme())
    }

    @Test fun eachThemeMapsToItsNightMode() {
        assertEquals(android.app.UiModeManager.MODE_NIGHT_YES, dev.murmur.ui.ThemeMode.nightMode(AppTheme.DARK))
        assertEquals(android.app.UiModeManager.MODE_NIGHT_NO, dev.murmur.ui.ThemeMode.nightMode(AppTheme.LIGHT))
        assertEquals("follow the phone", android.app.UiModeManager.MODE_NIGHT_AUTO, dev.murmur.ui.ThemeMode.nightMode(AppTheme.SYSTEM))
    }

    @Test fun unknownStoredValueIsDark() {
        assertEquals(AppTheme.DARK, AppTheme.fromKey("sepia"))
        assertEquals(AppTheme.DARK, AppTheme.fromKey(null))
    }
}
