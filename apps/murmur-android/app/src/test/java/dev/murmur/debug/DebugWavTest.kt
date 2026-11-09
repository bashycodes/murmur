package dev.flowclone.debug

import dev.flowclone.session.DictationState
import dev.flowclone.session.InsertTarget
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.File
import java.nio.file.Files

/** Debug "WAV as microphone": which files it may read and when it refuses to start. */
class DebugWavTest {
    private val files: File = Files.createTempDirectory("flowfiles").toFile().apply { deleteOnExit() }

    @Test fun readsOnlyFromTheDebugDirectory() {
        val inDir = File(files, "debug/wav-in").apply { mkdirs() }
        val ok = File(inDir, "a.wav").apply { writeBytes(Wav.encode(shortArrayOf(1))) }
        val kept = File(files, "debug/audio/2026-10-27T10-00-00.000Z.wav").apply { parentFile!!.mkdirs(); writeBytes(Wav.encode(shortArrayOf(1))) }
        File(files, "secret.xml").writeText("x")
        assertEquals(ok.canonicalFile, DebugWav.resolve(files, "files/debug/wav-in/a.wav"))
        assertEquals(ok.canonicalFile, DebugWav.resolve(files, "debug/wav-in/a.wav"))
        assertEquals("a kept dictation can be replayed", kept.canonicalFile, DebugWav.resolve(files, "files/debug/audio/2026-10-27T10-00-00.000Z.wav"))
        for (bad in listOf("files/secret.xml", "files/debug/../secret.xml", "/etc/passwd", "files/debug/wav-in/missing.wav", "", "files/debug/wav-in")) {
            assertNull(bad, DebugWav.resolve(files, bad))
        }
    }

    @Test fun refusesWhileBusyOrInAProtectedField() {
        assertNull(DebugWav.refusal(DictationState.Ready, InsertTarget.OK))
        assertNull(DebugWav.refusal(DictationState.Idle, null))
        assertNull(DebugWav.refusal(DictationState.Error(DictationState.ErrorKind.FailedToTranscribe), InsertTarget.OK))
        assertNotNull(DebugWav.refusal(DictationState.Recording(true, 0), InsertTarget.OK))
        assertNotNull(DebugWav.refusal(DictationState.Processing(true, 0), InsertTarget.OK))
        assertEquals("protected target", DebugWav.refusal(DictationState.Ready, InsertTarget.PROTECTED))
    }
}
