package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D104: tap dictations no longer end after silence by default; only a fresh install gets that ([EndpointDefault]). */
class EndpointDefaultTest {
    /** As `SettingsStore.loadEndpointMs` (the D22 flag set, as on any install that loaded settings since 0.2). */
    private fun MemKV.runs(): Int? {
        val stored = m[EndpointDefault.KEY] as Int?
        val ms = Settings.migrateEndpointMs(stored, migrated = getBoolean("endpoint_default_1500", false))
        return Settings(endpointSilenceMs = ms).handsFreeEndpointMs()
    }

    /** As `SettingsStore`: one emptiness check, then every keep step. */
    private fun MemKV.keepOld() = keepOldDefaults(this, empty = m.isEmpty())

    @Test fun freshEmptyStoreNeverAutoEnds() {
        val kv = MemKV()
        kv.keepOld()
        assertFalse("nothing written for the endpoint", kv.contains(EndpointDefault.KEY))
        assertTrue(kv.getBoolean(EndpointDefault.MIGRATED, false))
        assertNull("no endpoint_silence_ms sent", kv.runs())
        assertNull(Settings().handsFreeEndpointMs())
    }

    @Test fun anUpgradeThatNeverSavedKeepsOnePointFiveSeconds() {
        val kv = MemKV().apply { m["language_default_device"] = true; m["endpoint_default_1500"] = true }
        kv.keepOld()
        assertEquals(1500, kv.m[EndpointDefault.KEY])
        assertEquals(1500, kv.runs())
    }

    @Test fun anUpgradeThatAlreadyRanTheEarlierStepsStillKeepsIt() {
        // D103 / theme flags set on an earlier start: the store is not empty.
        val kv = MemKV().apply { m[StrengthDefault.MIGRATED] = true; m[ThemeDefault.MIGRATED] = true }
        kv.keepOld()
        assertEquals(1500, kv.m[EndpointDefault.KEY])
    }

    @Test fun aStoredValueIsNeverTouched() {
        for (v in listOf(0, 1500, 3000)) {
            val kv = MemKV().apply { m["endpoint_default_1500"] = true; m[EndpointDefault.KEY] = v }
            kv.keepOld()
            assertEquals(v, kv.m[EndpointDefault.KEY])
        }
    }

    @Test fun anUntouchedPreD22TwoSecondsStillMovesToOnePointFive() {
        val kv = MemKV().apply { m[EndpointDefault.KEY] = 2000 }
        kv.keepOld()
        assertEquals(1500, kv.runs())
    }

    @Test fun theFlagIsSetOnceAndLaterSavesAreKept() {
        val kv = MemKV().apply { m["language"] = "en" }
        kv.keepOld()
        kv.m.remove(EndpointDefault.KEY)
        kv.keepOld()
        assertFalse("not written again after the flag", kv.contains(EndpointDefault.KEY))
    }

    @Test fun restoringAnOldBackupKeepsOnePointFiveSeconds() {
        val kv = MemKV()
        kv.keepOld()
        // BackupIo.replacePrefs: clear, then write an older backup's keys (no flags, no endpoint).
        kv.edit { clear(); putString("language", "en") }
        kv.keepOld() // SettingsStore.load after the restore
        assertEquals(1500, kv.m[EndpointDefault.KEY])
    }
}
