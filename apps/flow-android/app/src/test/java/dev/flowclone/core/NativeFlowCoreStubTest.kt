package dev.flowclone.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Runs only with `-Pflow.jniStubDir=…` (tools/probes/jni_stub/run.sh), which puts a host
 * build of the C stub on java.library.path. Verifies the Kotlin side links to exactly the
 * symbols documented in JNI_CONTRACT.md and round-trips JSON through them.
 */
class NativeFlowCoreStubTest {
    private fun requireStub() = assumeTrue("jni stub not built", System.getProperty("flow.jniStub") == "1")

    @Test fun linksAndRoundTripsThroughEverySymbol() {
        requireStub()
        val lib = NativeFlowCore.loadLibrary()
        assertTrue(lib.toString(), lib is NativeFlowCore.LibraryState.Loaded)
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = "/m.bin"))
        core.startSession(SessionConfig(languages = listOf("en")))
        core.setContext(FieldContext(appId = "com.x"))
        assertTrue(core.pushAudio(ShortArray(1600) { 3000 }))
        assertEquals(null, core.partial())
        assertEquals(SessionRoute("local"), core.poll().route)
        val r = core.commit()
        assertEquals(CorePoll(), core.poll())
        assertEquals(StartInfo(route = SessionRoute("local")), core.startSessionInfo(SessionConfig(endpointSilenceMs = 2000)))
        core.cancel()
        assertEquals(r.toString(), CommitStatus.RAW_TRANSCRIPT, r.status)
        assertEquals("Stub text.", r.finalText)
        assertEquals(100L, r.timings.audioMs)
        assertEquals(listOf(LearnCandidate("wisper", "Wispr")), core.learnEdit("wisper", "Wispr"))
        core.cancel()
        assertFalse(core.pushAudio(ShortArray(10)))
        core.unload()
        assertEquals(CommitStatus.ERROR, core.commit().status)
    }

    @Test fun loadFailureSurfacesLastError() {
        requireStub()
        val e = runCatching { NativeFlowCore.loadModels(ModelConfig(asrPath = "/fail")) }.exceptionOrNull()
        assertTrue(e is CoreException)
        assertEquals("model load failed: /fail", e!!.message)
    }
}
