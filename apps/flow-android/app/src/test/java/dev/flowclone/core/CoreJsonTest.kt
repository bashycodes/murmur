package dev.flowclone.core

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Field names must match crates/flow-core serde (config.rs / engine.rs / dictionary.rs). */
class CoreJsonTest {
    @Test fun modelConfigUsesSnakeCaseAndWhisperBackend() {
        val j = JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/base.bin", threads = 4)))
        assertEquals("whisper", j.getString("asr_backend"))
        assertEquals("/m/base.bin", j.getString("asr_path"))
        assertEquals(4, j.getInt("threads"))
        assertTrue(j.isNull("llm_path"))
    }

    @Test fun modelConfigMockBackendIsExternallyTagged() {
        val j = JSONObject(CoreJson.encode(ModelConfig(asrPath = "", mockText = "hello")))
        assertEquals("hello", j.getJSONObject("asr_backend").getJSONObject("mock").getString("text"))
    }

    @Test fun sessionConfigMatchesCoreShape() {
        val cfg = SessionConfig(
            languages = listOf("en"),
            dictionary = listOf(DictEntry("wispr", "Wispr", false), DictEntry("sig", "Best,\nMe", true), DictEntry("Kubernetes")),
            cleanupLevel = CleanupLevel.NONE,
            rawMode = true,
            maxSamples = 300L * 16_000,
        )
        val j = JSONObject(CoreJson.encode(cfg))
        assertEquals("android", j.getString("platform"))
        assertEquals("none", j.getString("cleanup_level"))
        assertTrue(j.getBoolean("raw_mode"))
        assertEquals(4_800_000L, j.getLong("max_samples"))
        val d = j.getJSONArray("dictionary")
        assertEquals("Wispr", d.getJSONObject(0).getString("replacement"))
        assertTrue(d.getJSONObject(1).getBoolean("isSnippet"))
        assertFalse("pure vocabulary word omits replacement", d.getJSONObject(2).has("replacement"))
        assertEquals("formal", j.getJSONObject("styles").getString("personal"))
    }

    @Test fun fieldContextShape() {
        val j = JSONObject(CoreJson.encode(FieldContext(appId = "com.Slack", appCategory = "work_chat", before = "Hi ", screenWords = listOf("Ada"))))
        assertEquals("com.Slack", j.getString("app_id"))
        assertEquals("work_chat", j.getString("app_category"))
        assertEquals("Hi ", j.getString("before"))
        assertEquals("Ada", j.getJSONArray("screen_words").getString(0))
    }

    @Test fun decodesCoreCommitResult() {
        val json = """{"status":"RAW_TRANSCRIPT","asr_text":"hello world","final_text":"Hello world.",
            "language":"en","timings":{"audio_ms":1200,"asr_ms":300,"llm_ms":0,"postprocess_ms":1,"total_ms":320,"dropped_samples":0},
            "press_enter":false,"fallback_reason":"llm_off","truncated":false,"error":null}"""
        val r = CoreJson.decodeCommit(json)
        assertEquals(CommitStatus.RAW_TRANSCRIPT, r.status)
        assertEquals("Hello world.", r.finalText)
        assertEquals("en", r.language)
        assertEquals(320L, r.timings.totalMs)
        assertEquals("llm_off", r.fallbackReason)
        assertNull(r.error)
    }

    @Test fun malformedCommitJsonBecomesErrorResult() {
        val r = CoreJson.decodeCommit("not json")
        assertEquals(CommitStatus.ERROR, r.status)
        assertTrue(r.error!!.isNotEmpty())
    }

    @Test fun decodesLearnCandidates() {
        val c = CoreJson.decodeCandidates("""[{"original":"wisper","corrected":"Wispr"}]""")
        assertEquals(listOf(LearnCandidate("wisper", "Wispr")), c)
    }

    @Test fun dictionaryRoundTripsSharedExportFormat() {
        val entries = listOf(DictEntry("a", null, false), DictEntry("b", "B", false), DictEntry("c", "see", true))
        val json = CoreJson.encodeDictionary(entries)
        val arr = JSONArray(json)
        assertEquals(3, arr.length())
        assertEquals(entries, CoreJson.decodeDictionary(json))
    }

    @Test fun dictionaryImportToleratesMissingFieldsAndSkipsBlankWords() {
        val d = CoreJson.decodeDictionary("""[{"word":"x"},{"word":"  "},{"word":"y","replacement":"","isSnippet":false}]""")
        assertEquals(listOf(DictEntry("x"), DictEntry("y")), d)
    }
}

class FakeFlowCoreTest {
    @Test fun commitReturnsScriptedTextAfterAudio() {
        val core = FakeFlowCore(transcript = "hi there")
        core.startSession(SessionConfig())
        assertTrue(core.pushAudio(ShortArray(1600) { 1000 }))
        val r = core.commit()
        assertEquals(CommitStatus.RAW_TRANSCRIPT, r.status)
        assertEquals("hi there", r.finalText)
        assertEquals(1600, core.samplesPushed)
    }

    @Test fun silenceCommitsEmpty() {
        val core = FakeFlowCore(transcript = "hi")
        core.startSession(SessionConfig())
        core.pushAudio(ShortArray(1600))
        assertEquals(CommitStatus.EMPTY, core.commit().status)
    }

    @Test fun pushWithoutSessionIsRejected() = assertFalse(FakeFlowCore().pushAudio(ShortArray(10)))

    @Test fun cancelDiscardsSession() {
        val core = FakeFlowCore(transcript = "hi")
        core.startSession(SessionConfig())
        core.cancel()
        assertFalse(core.pushAudio(ShortArray(10) { 500 }))
    }
}

class NativeFlowCoreTest {
    @Test fun missingLibraryIsReportedNotThrown() {
        org.junit.Assume.assumeTrue(System.getProperty("flow.jniStub") == null)
        val r = NativeFlowCore.loadLibrary()
        assertTrue(r is NativeFlowCore.LibraryState.Unavailable)
        assertTrue((r as NativeFlowCore.LibraryState.Unavailable).message.contains("flow_ffi"))
    }
}
