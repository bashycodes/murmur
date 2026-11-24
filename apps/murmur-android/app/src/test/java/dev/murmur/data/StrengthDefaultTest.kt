package dev.murmur.data

import dev.murmur.core.CleanupLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The strength default moved Light → Medium: only a fresh install gets Medium ([StrengthDefault]). */
class StrengthDefaultTest {
    private fun MemKV.runs(): CleanupLevel =
        Settings.strengthOf(CleanupLevel.entries.firstOrNull { it.wire == getString("cleanup") } ?: CleanupLevel.NONE)

    private fun MemKV.keepOld() = StrengthDefault.keepOld(this, empty = m.isEmpty())

    @Test fun freshEmptyStoreRunsMedium() {
        val kv = MemKV()
        kv.keepOld()
        assertNull("nothing written for the strength", kv.getString("cleanup"))
        assertTrue(kv.getBoolean(StrengthDefault.MIGRATED, false))
        assertEquals(CleanupLevel.MEDIUM, kv.runs())
    }

    @Test fun storedNoneFromBeforeIsRewrittenToLight() {
        val kv = MemKV().apply { m["cleanup"] = "none"; m["llm_enabled"] = true }
        kv.keepOld()
        assertEquals("light", kv.getString("cleanup"))
        assertEquals(CleanupLevel.LIGHT, kv.runs())
    }

    @Test fun upgradeWithoutTheCleanupKeyKeepsLight() {
        // Only the language migration had written prefs; your server cleaned at Light (D36).
        val kv = MemKV().apply { m["language_default_device"] = true; m["language_default_keyboard"] = true; m["language"] = "keyboard" }
        kv.keepOld()
        assertEquals("light", kv.getString("cleanup"))
    }

    @Test fun aPickedStrengthIsNeverTouched() {
        for (l in listOf("light", "medium", "high")) {
            val kv = MemKV().apply { m["cleanup"] = l }
            kv.keepOld()
            assertEquals(l, kv.getString("cleanup"))
        }
    }

    @Test fun theFlagIsSetOnceAndLaterSavesAreKept() {
        val kv = MemKV().apply { m["cleanup"] = "none" }
        kv.keepOld()
        assertEquals("light", kv.getString("cleanup"))
        // After the move a NONE (never picked on a fresh install) is the new default, not rewritten again.
        kv.m["cleanup"] = "none"
        kv.keepOld()
        assertEquals("none", kv.getString("cleanup"))
        assertEquals(CleanupLevel.MEDIUM, kv.runs())
    }

    @Test fun restoringAnOldBackupKeepsLight() {
        // This install is on the new default; the backup is from before (no flag, no strength picked).
        val kv = MemKV()
        kv.keepOld()
        val meta = Backup.Meta("1.6.0", "android", "hybrid", "2026-11-01T10:00:00+02:00")
        val old = Backup.Data(settings = mapOf("language" to Backup.Pref.Str("en"), "llm_enabled" to Backup.Pref.Bool(true)))
        val restored = (Backup.decode(Backup.encode(meta, old)) as Backup.Parsed.Ok).data.settings!!
        // BackupIo.replacePrefs: clear, then write the backup's keys.
        kv.edit {
            clear()
            for ((k, v) in restored) when (v) {
                is Backup.Pref.Str -> putString(k, v.v)
                is Backup.Pref.Bool -> putBoolean(k, v.v)
                else -> error("unused here")
            }
        }
        kv.keepOld() // SettingsStore.load after the restore
        assertEquals("light", kv.getString("cleanup"))
        assertEquals(CleanupLevel.LIGHT, kv.runs())
    }
}
