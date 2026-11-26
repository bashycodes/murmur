package dev.murmur.data

import android.content.Context
import dev.murmur.MurmurApp
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** D114: live transcription is always on; a stored "Transcribe while I speak" is dropped once ([LiveTranscriptionAlways]). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class LiveTranscriptionAlwaysTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val sp get() = ctx.getSharedPreferences("murmur_settings", Context.MODE_PRIVATE)
    private val old = LiveTranscriptionAlways.OLD_KEY

    @After fun reset() { sp.edit().clear().commit() }

    @Test fun aSavedOffOrOnLoadsAndEndsUpLive() {
        for (v in listOf(false, true)) {
            sp.edit().clear().putString("language", "en").putBoolean(old, v).commit()
            val s = SettingsStore(ctx).load()
            assertEquals("$v", 800, s.toSessionConfig(emptyList()).segmentPauseMs)
            assertFalse("$v dropped", sp.contains(old))
            assertTrue(sp.getBoolean(LiveTranscriptionAlways.flag, false))
        }
    }

    @Test fun anOlderBackupRestoredLaterIsDroppedAgain() {
        val store = SettingsStore(ctx)
        store.save(store.load())
        assertTrue("every save sets the flag", sp.getBoolean(LiveTranscriptionAlways.flag, false))
        // A Restore replaces the prefs with an older file: its key comes back without the flag.
        sp.edit().clear().putString("language", "en").putBoolean(old, false).commit()
        assertEquals(800, store.load().toSessionConfig(emptyList()).segmentPauseMs)
        assertFalse(sp.contains(old))
    }

    @Test fun aFreshInstallOnlySetsTheFlag() {
        sp.edit().clear().commit()
        SettingsStore(ctx).load()
        assertFalse(sp.contains(old))
        assertTrue(sp.getBoolean(LiveTranscriptionAlways.flag, false))
    }
}
