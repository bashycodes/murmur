package dev.murmur.core

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Field names must match crates/murmur-core serde (config.rs / engine.rs / dictionary.rs). */
class CoreJsonTest {
    @Test fun modelConfigUsesSnakeCaseAndWhisperBackend() {
        val j = JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/base.bin", threads = 4)))
        assertEquals("whisper", j.getString("asr_backend"))
        assertEquals("/m/base.bin", j.getString("asr_path"))
        assertEquals(4, j.getInt("threads"))
        assertTrue(j.isNull("llm_path"))
        assertTrue("default plugin lookup", j.isNull("llm_lib"))
    }

    @Test fun modelConfigMockBackendIsExternallyTagged() {
        val j = JSONObject(CoreJson.encode(ModelConfig(asrPath = "", mockText = "hello")))
        assertEquals("hello", j.getJSONObject("asr_backend").getJSONObject("mock").getString("text"))
    }

    @Test fun sessionConfigMatchesCoreShape() {
        val cfg = SessionConfig(
            languages = listOf("en"),
            dictionary = listOf(DictEntry("tailscale", "Tailscale", false), DictEntry("sig", "Best,\nMe", true), DictEntry("Kubernetes")),
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
        assertEquals("Tailscale", d.getJSONObject(0).getString("replacement"))
        assertTrue(d.getJSONObject(1).getBoolean("isSnippet"))
        assertFalse("pure vocabulary word omits replacement", d.getJSONObject(2).has("replacement"))
        assertEquals("D27: chat buckets default to casual", "casual", j.getJSONObject("styles").getString("personal"))
        assertEquals("formal", j.getJSONObject("styles").getString("email"))
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

    /** 2026-10-24: blank_asr diagnostics; absent in older cores / servers → null. */
    @Test fun decodesSpeechAndPeakDiagnostics() {
        val r = CoreJson.decodeCommit("""{"status":"EMPTY","empty_reason":"blank_asr","timings":{"audio_ms":3800,"speech_ms":1200,"peak_dbfs":-31}}""")
        assertEquals(1200L, r.timings.speechMs)
        assertEquals(-31, r.timings.peakDbfs)
        val old = CoreJson.decodeCommit("""{"status":"EMPTY","timings":{"audio_ms":3800}}""")
        assertNull(old.timings.speechMs)
        assertNull(old.timings.peakDbfs)
    }

    @Test fun modelConfigSelectsParakeet() {
        val j = JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/parakeet", asrEngine = AsrEngine.PARAKEET)))
        assertEquals("parakeet", j.getString("asr_backend"))
        assertEquals("/m/parakeet", j.getString("asr_path"))
    }

    @Test fun sessionConfigSendsCoreP1Flags() {
        val j = JSONObject(CoreJson.encode(SessionConfig(spokenCleanup = false)))
        assertTrue(j.getBoolean("vad"))
        assertTrue(j.getBoolean("hallucination_guard"))
        assertFalse(j.getBoolean("spoken_cleanup"))
        assertTrue(j.getBoolean("llm_prefill"))
    }

    /** Shape produced by murmur-core core-p1 (`paste::PasteSegment` untagged, CR-2) + D17 `route`. */
    @Test fun decodesSegmentsPlainTextEmptyReasonAndRoute() {
        val json = """{"status":"FORMATTED","asr_text":"x","final_text":"@foo{{TAB}} bar","language":"en",
            "timings":{"audio_ms":1,"asr_ms":1,"llm_ms":0,"postprocess_ms":0,"total_ms":2,"dropped_samples":0,"asr_audio_ms":1},
            "press_enter":true,"fallback_reason":null,"truncated":false,"error":null,
            "segments":[{"text":"@foo"},{"key":"Tab"},{"text":" bar"},{"key":"Break"},{"key":"Enter"},{"key":"Future"},{"weird":1}],
            "plain_text":"@foo bar","empty_reason":null,
            "route":{"engine":"local","fallback":"connect_failed","server":"100.64.0.10:7313","rtt_ms":null,"queue_ms":3,"server_core":null}}"""
        val r = CoreJson.decodeCommit(json)
        assertEquals(
            listOf(
                PasteSegment.Text("@foo"), PasteSegment.Key(PasteKey.TAB), PasteSegment.Text(" bar"),
                PasteSegment.Key(PasteKey.BREAK), PasteSegment.Key(PasteKey.ENTER),
            ),
            r.segments,
        )
        assertEquals(r.segments, r.pastePlan())
        assertEquals("@foo bar", r.historyText)
        assertEquals(RouteInfo("local", "connect_failed", "100.64.0.10:7313", null, 3, null), r.route)
    }

    @Test fun preCoreP1ResultStillPastesFinalTextThenEnter() {
        val r = CoreJson.decodeCommit("""{"status":"RAW_TRANSCRIPT","asr_text":"a","final_text":"Send it.","press_enter":true}""")
        assertEquals(listOf(PasteSegment.Text("Send it."), PasteSegment.Key(PasteKey.ENTER)), r.pastePlan())
        assertEquals("Send it.", r.historyText)
        assertNull(r.route)
    }

    @Test fun emptyResultCarriesReason() {
        val r = CoreJson.decodeCommit("""{"status":"EMPTY","asr_text":"","final_text":"","segments":[],"plain_text":"","empty_reason":"no_speech"}""")
        assertEquals("no_speech", r.emptyReason)
        assertTrue(r.pastePlan().isEmpty())
    }

    @Test fun loadErrorKindsAreRecoveredFromStablePrefixes() {
        assertEquals("model_not_found", CoreException.kindOf("model not found: /x/y.bin"))
        assertEquals("llm_lib_not_found", CoreException.kindOf("llm plugin not found (libmurmur_llm.so)"))
        assertEquals("no_engine_enabled", CoreException.kindOf("no_engine_enabled: both off"))
        assertNull(CoreException.kindOf("something else"))
        assertEquals("load", CoreException.of("x", "load", "f").kind)
        assertEquals("model_not_found", CoreException.of("model not found: a", "error", "f").kind)
        assertEquals("f", CoreException.of(null, null, "f").message)
    }

    @Test fun malformedCommitJsonBecomesErrorResult() {
        val r = CoreJson.decodeCommit("not json")
        assertEquals(CommitStatus.ERROR, r.status)
        assertTrue(r.error!!.isNotEmpty())
    }

    @Test fun decodesLearnCandidates() {
        val c = CoreJson.decodeCandidates("""[{"original":"tailscail","corrected":"Tailscale"}]""")
        assertEquals(listOf(LearnCandidate("tailscail", "Tailscale")), c)
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

class FakeMurmurCoreTest {
    @Test fun commitReturnsScriptedTextAfterAudio() {
        val core = FakeMurmurCore(transcript = "hi there")
        core.startSession(SessionConfig())
        assertTrue(core.pushAudio(ShortArray(1600) { 1000 }))
        val r = core.commit()
        assertEquals(CommitStatus.RAW_TRANSCRIPT, r.status)
        assertEquals("hi there", r.finalText)
        assertEquals(1600, core.samplesPushed)
    }

    @Test fun silenceCommitsEmpty() {
        val core = FakeMurmurCore(transcript = "hi")
        core.startSession(SessionConfig())
        core.pushAudio(ShortArray(1600))
        assertEquals(CommitStatus.EMPTY, core.commit().status)
    }

    @Test fun pushWithoutSessionIsRejected() = assertFalse(FakeMurmurCore().pushAudio(ShortArray(10)))

    @Test fun cancelDiscardsSession() {
        val core = FakeMurmurCore(transcript = "hi")
        core.startSession(SessionConfig())
        core.cancel()
        assertFalse(core.pushAudio(ShortArray(10) { 500 }))
    }
}

class NativeMurmurCoreTest {
    @Test fun missingLibraryIsReportedNotThrown() {
        org.junit.Assume.assumeTrue(System.getProperty("murmur.jniStub") == null)
        val r = NativeMurmurCore.loadLibrary()
        assertTrue(r is NativeMurmurCore.LibraryState.Unavailable)
        assertTrue((r as NativeMurmurCore.LibraryState.Unavailable).message.contains("murmur_ffi"))
    }

    @Test fun endpointSilenceOnlyForHandsFree() {
        assertFalse("omitted, not null, for push-to-talk", org.json.JSONObject(CoreJson.encode(SessionConfig())).has("endpoint_silence_ms"))
        assertEquals(2000, org.json.JSONObject(CoreJson.encode(SessionConfig(endpointSilenceMs = 2000))).getInt("endpoint_silence_ms"))
    }

    // ---- D22 streaming ASR (docs/proposals/streaming-asr-shell-api.md) ----

    @Test fun segmentPauseOmittedWhenOffAndSentWhenOn() {
        assertFalse("omitted, not null: old cores/servers see today's config", JSONObject(CoreJson.encode(SessionConfig())).has("segment_pause_ms"))
        val j = JSONObject(CoreJson.encode(SessionConfig(segmentPauseMs = 800)))
        assertEquals(800, j.getInt("segment_pause_ms"))
        assertFalse("core default", j.has("segment_min_ms"))
        assertFalse("core default", j.has("segment_soft_max_ms"))
    }

    @Test fun parakeetDecodingOmittedForAutoAndSentOtherwise() {
        assertFalse(JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/pk", asrEngine = AsrEngine.PARAKEET))).has("parakeet_decoding"))
        assertEquals("greedy", JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/pk", asrEngine = AsrEngine.PARAKEET, parakeetDecoding = ParakeetDecoding.GREEDY))).getString("parakeet_decoding"))
        assertEquals("beam", JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m/pk", asrEngine = AsrEngine.PARAKEET, parakeetDecoding = ParakeetDecoding.BEAM))).getString("parakeet_decoding"))
        assertEquals(ParakeetDecoding.GREEDY, ParakeetDecoding.fromWire("greedy"))
        assertEquals(null, ParakeetDecoding.fromWire("nonsense"))
    }

    @Test fun decodesSegmentedTimingsAndClientMs() {
        val json = """{"status":"RAW_TRANSCRIPT","asr_text":"a","final_text":"a",
            "timings":{"audio_ms":30000,"asr_ms":400,"llm_ms":0,"postprocess_ms":1,"total_ms":520,"dropped_samples":0,
            "asr_audio_ms":29000,"wait_ms":120,"segments":4,"pre_asr_ms":2400},
            "route":{"engine":"remote","rtt_ms":4,"client_ms":610}}"""
        val r = CoreJson.decodeCommit(json)
        assertEquals(120L, r.timings.waitMs)
        assertEquals(4L, r.timings.segments)
        assertEquals(2400L, r.timings.preAsrMs)
        assertEquals(610L, r.route!!.clientMs)
    }

    @Test fun segmentedTimingsDefaultToZeroWhenAbsent() {
        val r = CoreJson.decodeCommit("""{"status":"RAW_TRANSCRIPT","timings":{"total_ms":5},"route":{"engine":"local"}}""")
        assertEquals(0L, r.timings.waitMs)
        assertEquals(0L, r.timings.segments)
        assertEquals(0L, r.timings.preAsrMs)
        assertNull(r.route!!.clientMs)
    }
}
