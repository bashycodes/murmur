package dev.flowclone.debug

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files

/** "Keep last dictation audio (debug)": WAV files next to the D21 log, linked by `audio`. */
class DebugAudioTest {
    private fun tmpDir(): File = Files.createTempDirectory("flowaudio").toFile().apply { deleteOnExit() }
    private val hour = 3_600_000L

    @Test fun wavIs16kMonoPcm16() {
        val pcm = shortArrayOf(1, -2, 32767, -32768, 5)
        val b = ByteBuffer.wrap(Wav.encode(pcm, 4)).order(ByteOrder.LITTLE_ENDIAN)
        fun tag(at: Int) = String(ByteArray(4) { b.get(at + it) }, Charsets.US_ASCII)
        assertEquals("RIFF", tag(0)); assertEquals("WAVE", tag(8)); assertEquals("fmt ", tag(12)); assertEquals("data", tag(36))
        assertEquals(36 + 8, b.getInt(4))
        assertEquals(16, b.getInt(16)); assertEquals(1, b.getShort(20).toInt()); assertEquals(1, b.getShort(22).toInt())
        assertEquals(16_000, b.getInt(24)); assertEquals(32_000, b.getInt(28))
        assertEquals(2, b.getShort(32).toInt()); assertEquals(16, b.getShort(34).toInt())
        assertEquals(8, b.getInt(40))
        assertEquals(44 + 8, b.capacity())
        assertArrayEquals(shortArrayOf(1, -2, 32767, -32768), ShortArray(4) { b.getShort(44 + 2 * it) })
    }

    @Test fun nameIsTheDebugLogTsWithoutColons() {
        assertEquals("2025-10-23T10-15-02.123Z.wav", DebugAudioStore.nameFor(1_761_214_502_123))
        assertEquals("2025-10-23T10:15:02.123Z", DebugFormat.ts(1_761_214_502_123))
    }

    @Test fun keepsTheNewestTen() {
        var now = 1_761_214_502_123
        val store = DebugAudioStore(tmpDir(), clock = { now })
        repeat(13) { i -> store.save(now, ShortArray(160) { i.toShort() }, 160); now += 1_000 }
        val names = store.list()
        assertEquals(10, names.size)
        assertEquals(DebugAudioStore.nameFor(now - 1_000), names.first())
        assertEquals(DebugAudioStore.nameFor(now - 10_000), names.last())
        assertEquals(44 + 320L, File(store.dir, names.first()).length())
    }

    @Test fun filesOlderThan24hAreDeleted() {
        var now = 1_761_214_502_123
        val store = DebugAudioStore(tmpDir(), clock = { now })
        store.save(now, ShortArray(16), 16)
        now += 23 * hour
        store.save(now, ShortArray(16), 16)
        now += 2 * hour
        store.prune()
        assertEquals(listOf(DebugAudioStore.nameFor(now - 2 * hour)), store.list())
        now += 25 * hour
        store.prune()
        assertTrue(store.list().isEmpty())
    }

    @Test fun clearDeletesEverythingAndIgnoresOtherFiles() {
        val dir = tmpDir()
        val store = DebugAudioStore(dir, clock = { 1_761_214_502_123 })
        store.save(1_761_214_502_123, ShortArray(16), 16)
        File(store.dir, "notes.txt").writeText("x")
        assertEquals(1, store.list().size)
        store.clear()
        assertTrue(store.list().isEmpty())
        assertTrue("only our WAVs", File(store.dir, "notes.txt").exists())
    }

    @Test fun protectedTargetsNeverKeepAudio() {
        val t = DictationTrace(tsMs = 0, mode = "tap", target = "com.example.notes", insert = InsertOutcome(InsertOutcome.INSERTED))
        assertTrue(DebugRedact.keepsAudio(t, protectedField = false, protectedApp = false))
        assertFalse(DebugRedact.keepsAudio(t, protectedField = true, protectedApp = false))
        assertFalse(DebugRedact.keepsAudio(t, protectedField = false, protectedApp = true))
        assertFalse(DebugRedact.keepsAudio(t.copy(insert = InsertOutcome(InsertOutcome.TARGET_PROTECTED)), false, false))
    }

    @Test fun audioFieldIsOnTheLine() {
        val t = DictationTrace(tsMs = 0, mode = "tap", insert = InsertOutcome(InsertOutcome.INSERTED))
        assertEquals("a.wav", org.json.JSONObject(DebugFormat.dictation(t.copy(audio = "a.wav"))).getString("audio"))
        assertTrue(org.json.JSONObject(DebugFormat.dictation(t)).isNull("audio"))
        assertNull(t.audio)
    }
}
