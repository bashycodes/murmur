package dev.murmur.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D28 picker data: murmur-cloud `PickerRow` JSON → rows, the status line, and the filter JSON. */
class CloudCatalogTest {
    private val rowsJson = requireNotNull(javaClass.classLoader!!.getResource("openrouter_picker_stt.json")).readText()

    @Test fun decodesPickerRows() {
        val rows = CloudCatalog.decodeRows(rowsJson)
        assertEquals(2, rows.size)
        val w = rows[0]
        assertEquals("openai/whisper-large-v3-turbo", w.id); assertEquals("transcriptions", w.sttPath)
        assertTrue(w.zdr && w.fast && w.recommended); assertFalse(w.chatAudio); assertNull(w.allowedByAccount)
        assertEquals("openai/whisper-large-v3-turbo · \$0.012/h audio", CloudCatalog.meta(w, CloudCatalog.Use.STT))
        assertEquals("ZDR · fast", CloudCatalog.badgeLine(w))
        val g = rows[1]
        assertTrue(g.chatAudio); assertFalse(g.fast); assertEquals(false, g.allowedByAccount)
        assertEquals("ZDR · may paraphrase · blocked by account", CloudCatalog.badgeLine(g))
        assertEquals("google/gemini-3.5-flash-lite · ≈ \$0.035/h + output · 1M context", CloudCatalog.meta(g, CloudCatalog.Use.CLEANUP))
        assertEquals(emptyList<CloudCatalog.Row>(), CloudCatalog.decodeRows("not json"))
        assertEquals(emptyList<CloudCatalog.Row>(), CloudCatalog.decodeRows("[{\"name\":\"no id\"}]"))
    }

    @Test fun filtersJsonMatchesRust() {
        val d = JSONObject(CloudCatalog.Filters().toJson())
        assertTrue("ZDR only is the default", d.getBoolean("zdr_only"))
        assertFalse(d.getBoolean("fast_only")); assertFalse(d.getBoolean("free_only")); assertTrue(d.isNull("stt_path"))
        assertEquals("chat_audio", JSONObject(CloudCatalog.Filters(sttPath = "chat_audio").toJson()).getString("stt_path"))
    }

    @Test fun statusLines() {
        val clock = { _: Long -> "10:42" }
        val live = CloudCatalog.decodeStatus("""{"source":"Live","fetched_at_ms":1000,"snapshot_date":null,"models":520,"stt":61,"cleanup":455}""")
        assertEquals("Live list · refreshed 10:42 · 61 speech, 455 text models", CloudCatalog.sourceLine(live, 2000, clock))
        val cache = CloudCatalog.decodeStatus("""{"source":"Cache","fetched_at_ms":0,"models":1,"stt":1,"cleanup":0}""")
        assertEquals("Cached list · 3 h old · 1 speech, 0 text models", CloudCatalog.sourceLine(cache, 3 * 3_600_000L + 5, clock))
        val baked = CloudCatalog.decodeStatus("""{"source":"Baked","fetched_at_ms":0,"snapshot_date":"2026-10-24","models":30,"stt":14,"cleanup":16,"error":"connect_failed"}""")
        assertEquals("Offline list (snapshot 2026-10-24) · 14 speech, 16 text models · refresh failed (no internet)", CloudCatalog.sourceLine(baked, 0, clock))
        assertEquals("cloud_not_configured", CloudCatalog.decodeStatus("""{"error":"cloud_not_configured"}""").error)
        assertEquals("error", CloudCatalog.decodeStatus("garbage").error)
    }

    @Test fun contextLabels() {
        assertEquals("131k context", CloudCatalog.contextLabel(131_072))
        assertEquals("2M context", CloudCatalog.contextLabel(2_000_000))
        assertEquals("openai/x · chat model (may paraphrase)", CloudCatalog.modelSummary("openai/x", "chat_audio"))
        assertEquals("m/x", CloudCatalog.modelSummary("m/x"))
    }
}
