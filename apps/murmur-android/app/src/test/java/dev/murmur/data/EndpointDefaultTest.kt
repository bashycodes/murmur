package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D104: tap dictations no longer end after silence by default; only a fresh install gets that ([EndpointDefault]). */
class EndpointDefaultTest {
    /** What a hands-free session sends after `SettingsStore.load` reads the endpoint. */
    private fun MemKV.runs(): Int? = Settings(endpointSilenceMs = EndpointDefault.load(this)).handsFreeEndpointMs()

    /** As `SettingsStore`: one emptiness check, then every keep step. */
    private fun MemKV.keepOld() = keepOldDefaults(this, empty = m.isEmpty())

    @Test fun freshEmptyStoreNeverAutoEnds() {
        val kv = MemKV()
        kv.keepOld()
        assertFalse("nothing written for the endpoint", kv.contains(EndpointDefault.key))
        assertTrue(kv.getBoolean(EndpointDefault.flag, false))
        assertNull("no endpoint_silence_ms sent", kv.runs())
        assertNull(Settings().handsFreeEndpointMs())
    }

    @Test fun anUpgradeThatNeverSavedKeepsOnePointFiveSeconds() {
        val kv = MemKV().apply { m["language_default_device"] = true; m[EndpointDefault.D22_MIGRATED] = true }
        kv.keepOld()
        assertEquals(1500, kv.m[EndpointDefault.key])
        assertEquals(1500, kv.runs())
    }

    @Test fun anUpgradeThatAlreadyRanTheEarlierStepsStillKeepsIt() {
        // D103 / theme flags set on an earlier start: the store is not empty.
        val kv = MemKV().apply { m[StrengthDefault.MIGRATED] = true; m[ThemeDefault.flag] = true }
        kv.keepOld()
        assertEquals(1500, kv.m[EndpointDefault.key])
    }

    @Test fun everyStepHasItsOwnFlag() {
        val flags = OLD_DEFAULTS.map { it.flag }
        assertEquals(listOf(StrengthDefault.MIGRATED, "theme_default_dark", "endpoint_default_off", "live_transcription_always"), flags)
    }

    @Test fun aStoredValueIsNeverTouched() {
        for (v in listOf(0, 1500, 3000)) {
            val kv = MemKV().apply { m[EndpointDefault.D22_MIGRATED] = true; m[EndpointDefault.key] = v }
            kv.keepOld()
            assertEquals(v, kv.m[EndpointDefault.key])
        }
    }

    @Test fun anUntouchedPreD22TwoSecondsStillMovesToOnePointFive() {
        val kv = MemKV().apply { m[EndpointDefault.key] = 2000 }
        kv.keepOld()
        assertEquals(1500, kv.runs())
    }

    @Test fun theFlagIsSetOnceAndLaterSavesAreKept() {
        val kv = MemKV().apply { m["language"] = "en" }
        kv.keepOld()
        kv.m.remove(EndpointDefault.key)
        kv.keepOld()
        assertFalse("not written again after the flag", kv.contains(EndpointDefault.key))
    }

    @Test fun restoringAnOldBackupKeepsOnePointFiveSeconds() {
        val kv = MemKV()
        kv.keepOld()
        // BackupIo.replacePrefs: clear, then write an older backup's keys (no flags, no endpoint).
        kv.edit { clear(); putString("language", "en") }
        kv.keepOld() // SettingsStore.load after the restore
        assertEquals(1500, kv.m[EndpointDefault.key])
    }
}
