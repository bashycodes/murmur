package dev.murmur.core

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * D90: [TranslateOutcome.decode] against murmur-core's translation wire vectors
 * (crates/murmur-core/fixtures/translate/translate_results.json, on the test classpath; the Rust
 * test crates/murmur-core/tests/translate_vectors.rs checks the encoder against the same file).
 */
class TranslateVectorsTest {
    private val fixture by lazy {
        JSONObject(javaClass.classLoader!!.getResource("translate/translate_results.json")!!.readText())
    }

    @Test fun everyVectorDecodesToItsOutcome() {
        val cases = fixture.getJSONArray("cases")
        val kinds = mutableSetOf<String>()
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val input = c.getJSONObject("input")
            val want = c.getJSONObject("json")
            val got = TranslateOutcome.decode(want.toString())
            if (input.has("ok")) {
                assertEquals(TranslateOutcome.Text(input.getString("ok"), input.getString("target"), input.getBoolean("on_device")), got)
            } else {
                got as TranslateOutcome.Failed
                assertEquals(input.getString("err"), got.kind)
                assertEquals(want.getString("error"), got.message)
                assertEquals(input.optString("target", ""), got.target)
                assertEquals(
                    "the bar's text is the core's TranslateError::notice",
                    c.getString("notice"),
                    dev.murmur.bubble.PreviewContent.notTranslated(got.kind, got.target),
                )
                kinds += got.kind
            }
        }
        val all = setOf(
            TranslateOutcome.NO_TEXT, TranslateOutcome.TEXT_TOO_LONG, TranslateOutcome.NO_TARGET,
            TranslateOutcome.NO_LLM, TranslateOutcome.TIMEOUT, TranslateOutcome.EMPTY,
            TranslateOutcome.NOT_TRANSLATED, TranslateOutcome.BACKEND,
        )
        assertEquals("the Kotlin kind constants are the core's", all, kinds)
    }

    @Test fun theShellsNoLlmTextIsTheCores() {
        val cases = fixture.getJSONArray("cases")
        val noLlm = (0 until cases.length()).map { cases.getJSONObject(it).getJSONObject("json") }
            .first { it.optString("kind") == TranslateOutcome.NO_LLM }
        assertEquals(noLlm.getString("error"), TranslateOutcome.NO_LLM_MESSAGE)
    }

    @Test fun garbageNeverTypesText() {
        for (bad in listOf(null, "", "not json", """{"ok":true}""", """{"ok":true,"text":""}""", "{}")) {
            assertTrue(bad.toString(), TranslateOutcome.decode(bad) is TranslateOutcome.Failed)
        }
    }

    @Test fun theRequestLeavesAnAutomaticTargetOut() {
        val auto = JSONObject(TranslateOutcome.request("hello there", "en", null, listOf("he", "en"), emptyList()))
        assertFalse(auto.has("target"))
        assertEquals("he", auto.getJSONArray("installed").getString(0))
        assertEquals(0, auto.getJSONArray("dictionary").length())
        val fixed = JSONObject(
            TranslateOutcome.request("hello there", null, "fr", emptyList(), listOf(DictEntry("Zephyr"))),
        )
        assertEquals("fr", fixed.getString("target"))
        assertFalse(fixed.has("source"))
        assertEquals("Zephyr", fixed.getJSONArray("dictionary").getJSONObject(0).getString("word"))
    }
}
