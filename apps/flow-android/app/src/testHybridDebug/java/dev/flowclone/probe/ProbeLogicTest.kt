package dev.flowclone.probe

import android.speech.SpeechRecognizer
import com.google.mlkit.genai.common.FeatureStatus
import com.google.mlkit.genai.common.GenAiException
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** English 1 probe: the pure parts (status / error names, timing, JSONL record, WAV → 16 kHz). */
class ProbeLogicTest {
    // ---- names pinned to the library / platform constants ----

    @Test fun featureStatusNamesMatchMlKit() {
        assertEquals("UNAVAILABLE", ProbeCodes.featureStatus(FeatureStatus.UNAVAILABLE))
        assertEquals("DOWNLOADABLE", ProbeCodes.featureStatus(FeatureStatus.DOWNLOADABLE))
        assertEquals("DOWNLOADING", ProbeCodes.featureStatus(FeatureStatus.DOWNLOADING))
        assertEquals("AVAILABLE", ProbeCodes.featureStatus(FeatureStatus.AVAILABLE))
        assertEquals("UNKNOWN(42)", ProbeCodes.featureStatus(42))
    }

    @Test fun genAiErrorNamesMatchMlKit() {
        val codes = mapOf(
            GenAiException.ErrorCode.BACKGROUND_USE_BLOCKED to "BACKGROUND_USE_BLOCKED",
            GenAiException.ErrorCode.BUSY to "BUSY",
            GenAiException.ErrorCode.NOT_AVAILABLE to "NOT_AVAILABLE",
            GenAiException.ErrorCode.PER_APP_BATTERY_USE_QUOTA_EXCEEDED to "PER_APP_BATTERY_USE_QUOTA_EXCEEDED",
            GenAiException.ErrorCode.AICORE_INCOMPATIBLE to "AICORE_INCOMPATIBLE",
            GenAiException.ErrorCode.NEEDS_SYSTEM_UPDATE to "NEEDS_SYSTEM_UPDATE",
            GenAiException.ErrorCode.NOT_ENOUGH_DISK_SPACE to "NOT_ENOUGH_DISK_SPACE",
            GenAiException.ErrorCode.REQUEST_TOO_SMALL to "REQUEST_TOO_SMALL",
            GenAiException.ErrorCode.CANCELLED to "CANCELLED",
            GenAiException.ErrorCode.UNKNOWN to "UNKNOWN",
        )
        for ((c, n) in codes) assertEquals(n, ProbeCodes.genAiError(c))
        assertEquals("BACKGROUND_USE_BLOCKED", ProbeCodes.genAiError(30))
        assertEquals("CODE_9999", ProbeCodes.genAiError(9999))
    }

    @Test fun platformErrorNamesMatchSpeechRecognizer() {
        assertEquals("ERROR_NO_MATCH", ProbeCodes.platformError(SpeechRecognizer.ERROR_NO_MATCH))
        assertEquals("ERROR_INSUFFICIENT_PERMISSIONS", ProbeCodes.platformError(SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS))
        assertEquals("ERROR_LANGUAGE_UNAVAILABLE", ProbeCodes.platformError(SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE))
        assertEquals("ERROR_LANGUAGE_NOT_SUPPORTED", ProbeCodes.platformError(SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED))
        assertEquals("ERROR_CANNOT_CHECK_SUPPORT", ProbeCodes.platformError(SpeechRecognizer.ERROR_CANNOT_CHECK_SUPPORT))
        assertEquals("ERROR_SERVER_DISCONNECTED", ProbeCodes.platformError(SpeechRecognizer.ERROR_SERVER_DISCONNECTED))
        assertEquals("ERROR_99", ProbeCodes.platformError(99))
    }

    @Test fun platformSupportPicksTheBestStateAndNormalisesTags() {
        assertEquals("installed", ProbeCodes.platformSupport("en-US", listOf("en_us"), listOf("en-US"), listOf("en-US"), emptyList()))
        assertEquals("pending", ProbeCodes.platformSupport("en-US", emptyList(), listOf("EN-us"), emptyList(), emptyList()))
        assertEquals("supported", ProbeCodes.platformSupport("en-US", listOf("de-DE"), emptyList(), listOf("en-US"), emptyList()))
        assertEquals("online", ProbeCodes.platformSupport("en-US", emptyList(), emptyList(), emptyList(), listOf("en-US")))
        assertEquals("unsupported", ProbeCodes.platformSupport("en-US", listOf("en-GB"), emptyList(), emptyList(), emptyList()))
    }

    @Test fun downloadProgressText() {
        assertEquals("45 % (54.0 / 120.0 MB)", ProbeCodes.downloadProgress(54_000_000, 120_000_000))
        assertEquals("100 % (130.0 / 120.0 MB)", ProbeCodes.downloadProgress(130_000_000, 120_000_000))
        assertEquals("3.5 MB", ProbeCodes.downloadProgress(3_500_000, null))
    }

    // ---- timing ----

    @Test fun latencyIsEndOfAudioToFinalText() {
        val t = ProbeTiming(startAt = 1_000, firstPartialAt = 1_400, audioEndAt = 5_000, lastFinalAt = 5_350, completedAt = 5_400)
        assertEquals(350L, t.latencyMs)
        assertEquals(400L, t.completedMs)
        assertEquals(400L, t.firstPartialMs)
        // Streaming: the final text came before the audio ended.
        assertEquals(-200L, t.copy(lastFinalAt = 4_800).latencyMs)
        assertNull(ProbeTiming(startAt = 0, audioEndAt = 10).latencyMs)
        assertNull(ProbeTiming(startAt = 0, lastFinalAt = 10).latencyMs)
    }

    // ---- JSONL record ----

    private fun record(text: String = "Run pytest.\nThen deploy.", errorCode: Int? = null, error: String? = null) = ProbeRecord(
        tsMs = 1_700_000_000_000, appBuild = "0.9.0 abc1234", engine = "genai", mode = "advanced", source = "clip_tts",
        trigger = "overlay", probeOnTop = false, status = "AVAILABLE",
        outcome = ProbeRecord.outcome(text, errorCode, false), text = text, partials = 3,
        timing = ProbeTiming(startAt = 0, firstPartialAt = 300, audioEndAt = 4_000, lastFinalAt = 4_250, completedAt = 4_300),
        audioMs = 4_000, errorCode = errorCode, error = error,
        extras = mapOf("locale" to "en-US", "biasing" to listOf("pytest", "UUID4"), "base_model" to null),
    )

    @Test fun recordIsOneJsonLineWithEveryField() {
        val line = record().toJsonLine()
        assertFalse("one line", line.contains('\n'))
        val o = JSONObject(line)
        assertEquals("genai", o.getString("engine"))
        assertEquals("advanced", o.getString("mode"))
        assertEquals("clip_tts", o.getString("source"))
        assertEquals("overlay", o.getString("trigger"))
        assertFalse(o.getBoolean("probe_on_top"))
        assertEquals("AVAILABLE", o.getString("status"))
        assertEquals("ok", o.getString("outcome"))
        assertEquals("Run pytest.\nThen deploy.", o.getString("text"))
        assertEquals(3, o.getInt("partials"))
        assertEquals(300L, o.getLong("first_partial_ms"))
        assertEquals(250L, o.getLong("latency_ms"))
        assertEquals(300L, o.getLong("completed_ms"))
        assertEquals(4_000L, o.getLong("audio_ms"))
        assertTrue(o.isNull("error_code"))
        assertTrue(o.isNull("error"))
        assertEquals("en-US", o.getString("locale"))
        assertEquals(JSONArray(listOf("pytest", "UUID4")).toString(), o.getJSONArray("biasing").toString())
        assertTrue(o.isNull("base_model"))
    }

    @Test fun recordCarriesTheErrorCodeAndNullMode() {
        val r = record(text = "", errorCode = 30, error = "BACKGROUND_USE_BLOCKED: app not in foreground")
            .copy(engine = "platform", mode = null, timing = ProbeTiming(startAt = 0))
        val o = JSONObject(r.toJsonLine())
        assertEquals("error", o.getString("outcome"))
        assertEquals(30, o.getInt("error_code"))
        assertTrue(o.getString("error").startsWith("BACKGROUND_USE_BLOCKED"))
        assertTrue(o.isNull("mode"))
        assertTrue(o.isNull("latency_ms"))
    }

    @Test fun outcomeWords() {
        assertEquals("ok", ProbeRecord.outcome("hi", null, false))
        assertEquals("ok", ProbeRecord.outcome("partial text kept", 9, false))
        assertEquals("error", ProbeRecord.outcome(" ", 9, false))
        assertEquals("empty", ProbeRecord.outcome("", null, false))
        assertEquals("cancelled", ProbeRecord.outcome("hi", null, true))
    }

    // ---- WAV → 16 kHz mono ----

    private fun wav(pcm: ShortArray, rate: Int, channels: Int, format: Int = 1, bits: Int = 16, dataSize: Int? = null): ByteArray {
        val data = pcm.size * 2
        val b = ByteBuffer.allocate(44 + data).order(ByteOrder.LITTLE_ENDIAN)
        b.put("RIFF".toByteArray()).putInt(36 + data).put("WAVE".toByteArray())
        b.put("fmt ".toByteArray()).putInt(16).putShort(format.toShort()).putShort(channels.toShort()).putInt(rate)
            .putInt(rate * channels * 2).putShort((channels * 2).toShort()).putShort(bits.toShort())
        b.put("data".toByteArray()).putInt(dataSize ?: data)
        for (s in pcm) b.putShort(s)
        return b.array()
    }

    @Test fun readsAnyRateAndChannelCount() {
        val w = ProbePcm.readWav(wav(shortArrayOf(100, 300, -100, -300), 22_050, 2))
        assertEquals(22_050, w.rate); assertEquals(2, w.channels)
        assertEquals(listOf<Short>(200, -200), ProbePcm.downmix(w.pcm, w.channels).toList())
    }

    @Test fun streamingTtsDataSizeTakesTheRest() {
        val w = ProbePcm.readWav(wav(shortArrayOf(1, 2, 3), 16_000, 1, dataSize = 0))
        assertEquals(listOf<Short>(1, 2, 3), w.pcm.toList())
        val w2 = ProbePcm.readWav(wav(shortArrayOf(1, 2, 3), 16_000, 1, dataSize = -1))
        assertEquals(3, w2.pcm.size)
    }

    @Test(expected = IllegalArgumentException::class) fun rejectsFloatWav() {
        ProbePcm.readWav(wav(shortArrayOf(1, 2), 16_000, 1, format = 3))
    }

    @Test(expected = IllegalArgumentException::class) fun rejects8Bit() {
        ProbePcm.readWav(wav(shortArrayOf(1, 2), 16_000, 1, bits = 8))
    }

    @Test(expected = IllegalArgumentException::class) fun rejectsNonWav() {
        ProbePcm.readWav("hello, not a wav file".toByteArray())
    }

    @Test fun resamplesTo16k() {
        val one = ShortArray(24_000) { 1000 }
        val out = ProbePcm.resample(one, 24_000)
        assertEquals(16_000, out.size)
        assertTrue(out.all { it == 1000.toShort() })
        assertEquals(1_000L, ProbePcm.durationMs(out.size))
        val ramp = ShortArray(8) { (it * 100).toShort() }
        assertEquals(listOf<Short>(0, 50, 100, 150, 200, 250, 300, 350, 400, 450, 500, 550, 600, 650, 700, 700), ProbePcm.resample(ramp, 8_000).toList())
        assertTrue(ProbePcm.resample(one, 16_000) === one)
    }

    @Test fun to16kMonoEndToEnd() {
        val stereo44k = ShortArray(44_100 * 2) { 500 }
        val pcm = ProbePcm.to16kMono(ProbePcm.Wav(stereo44k, 44_100, 2))
        assertEquals(16_000, pcm.size)
    }

    @Test fun littleEndianBytes() {
        assertEquals(listOf<Byte>(0x34, 0x12, 0xFF.toByte(), 0xFF.toByte()), ProbePcm.toLeBytes(shortArrayOf(0x1234, -1)).toList())
        assertEquals(listOf<Byte>(0xFF.toByte(), 0xFF.toByte()), ProbePcm.toLeBytes(shortArrayOf(0x1234, -1), 1, 2).toList())
    }

    // ---- folder batch ----

    @Test fun batchRunsOnlyWavsInNameOrder() {
        assertEquals(
            listOf("a02.WAV", "b01.wav", "s10.wav"),
            ProbeBatchPlan.wavs(listOf("s10.wav", "notes.txt", "b01.wav", ".hidden.wav", "a02.WAV", "c03.m4a")),
        )
    }

    @Test fun clipIdIsTheFileNameWithoutWav() {
        assertEquals("s01", ProbeBatchPlan.clipId("s01.wav"))
        assertEquals("2026-01-02T03-04-05.678Z", ProbeBatchPlan.clipId("2026-01-02T03-04-05.678Z.WAV"))
        assertEquals("raw.pcm", ProbeBatchPlan.clipId("raw.pcm"))
    }

    @Test fun batchSetsSkipTheResultsFolderAndHonourTheFilter() {
        val dirs = listOf("results", "set-b", "set-a", ".tmp")
        assertEquals(listOf("set-a", "set-b"), ProbeBatchPlan.sets(dirs, ""))
        assertEquals(listOf("set-b"), ProbeBatchPlan.sets(dirs, " set-b , missing"))
        assertEquals(emptyList<String>(), ProbeBatchPlan.sets(dirs, "results"))
    }

    @Test fun batchResultsFileIsPerSession() {
        assertEquals("batch-20260102T030405Z.jsonl", ProbeBatchPlan.resultsName("20260102T030405Z"))
    }

    @Test fun batchWatchdogWaitsForTheClipWhenPacedOnly() {
        assertEquals(70_000L, ProbeBatchPlan.watchdogMs(10_000, realTime = true))
        assertEquals(60_000L, ProbeBatchPlan.watchdogMs(10_000, realTime = false))
    }

    @Test fun batchLineCarriesIdTextAndTimings() {
        val r = record().copy(extras = mapOf("segmented" to true, "last_partial" to null, "biasing" to listOf("x")))
        val line = ProbeBatchPlan.line("set-a", "clip-01", r)
        assertFalse("one line", line.contains('\n'))
        val o = JSONObject(line)
        assertEquals("set-a", o.getString("set"))
        assertEquals("clip-01", o.getString("id"))
        assertEquals("genai", o.getString("engine"))
        assertEquals("advanced", o.getString("mode"))
        assertEquals("Run pytest.\nThen deploy.", o.getString("text"))
        assertEquals(300L, o.getLong("first_partial_ms"))
        assertEquals(250L, o.getLong("latency_ms"))
        assertEquals(300L, o.getLong("completed_ms"))
        assertEquals("ok", o.getString("outcome"))
        assertTrue(o.isNull("error"))
        assertEquals(4_000L, o.getLong("audio_ms"))
        assertTrue(o.getBoolean("segmented"))
        assertTrue(o.isNull("last_partial"))
        assertEquals("[\"x\"]", o.getJSONArray("biasing").toString())
        // Same keys as the run's probe-english line, so the two never drift.
        val single = JSONObject(r.toJsonLine())
        assertEquals(single.keys().asSequence().toSet() + setOf("set", "id"), o.keys().asSequence().toSet())
    }

    @Test fun firstNonBlankSkipsEmptyLeadingHypotheses() {
        assertEquals("run the tests", ProbeCodes.firstNonBlank(listOf("", "  ", null, " run the tests ", "other")))
        assertEquals("", ProbeCodes.firstNonBlank(listOf("", null)))
        assertEquals("", ProbeCodes.firstNonBlank(emptyList()))
    }
}
