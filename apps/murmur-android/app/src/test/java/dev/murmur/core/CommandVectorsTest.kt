package dev.murmur.core

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [CommandOutcome.decode] against murmur-core's Command Mode wire vectors
 * (crates/murmur-core/fixtures/command/command_results.json, on the test classpath; the Rust
 * test crates/murmur-core/tests/command_vectors.rs checks the encoder against the same file).
 */
class CommandVectorsTest {
    private val fixture by lazy {
        JSONObject(javaClass.classLoader!!.getResource("command/command_results.json")!!.readText())
    }

    @Test fun everyVectorDecodesToItsOutcome() {
        val cases = fixture.getJSONArray("cases")
        val kinds = mutableSetOf<String>()
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val input = c.getJSONObject("input")
            val want = c.getJSONObject("json")
            val got = CommandOutcome.decode(want.toString())
            if (input.has("ok")) {
                assertEquals(CommandOutcome.Text(input.getString("ok")), got)
            } else {
                got as CommandOutcome.Failed
                assertEquals(input.getString("err"), got.kind)
                assertEquals(want.getString("error"), got.message)
                kinds += got.kind
            }
        }
        val all = setOf(
            CommandOutcome.NO_INSTRUCTION, CommandOutcome.SELECTION_TOO_LONG, CommandOutcome.NO_LLM,
            CommandOutcome.SELECTION_NOT_ALLOWED, CommandOutcome.TIMEOUT, CommandOutcome.EMPTY,
            CommandOutcome.LANGUAGE_CHANGED, CommandOutcome.BACKEND,
        )
        assertEquals("the Kotlin kind constants are the core's", all, kinds)
    }

    @Test fun theShellsNoLlmTextIsTheCores() {
        val cases = fixture.getJSONArray("cases")
        val noLlm = (0 until cases.length()).map { cases.getJSONObject(it).getJSONObject("json") }
            .first { it.optString("kind") == CommandOutcome.NO_LLM }
        assertEquals(noLlm.getString("error"), CommandOutcome.NO_LLM_MESSAGE)
    }

    /** D78: the pill names the active Cloud provider (and LiteLLM's host), else "the cloud". */
    @Test fun theSelectionRefusalNamesTheActiveProvider() {
        val cases = fixture.getJSONArray("cases")
        val refusals = (0 until cases.length()).map { cases.getJSONObject(it) }
            .filter { it.getJSONObject("input").optString("err") == CommandOutcome.SELECTION_NOT_ALLOWED }
            .associate { it.getJSONObject("input").optString("to") to CommandOutcome.decode(it.getJSONObject("json").toString()) }
        assertEquals(setOf("OpenRouter", "LiteLLM (10.0.0.5)", ""), refusals.keys)
        for ((to, got) in refusals) {
            got as CommandOutcome.Failed
            val where = to.ifEmpty { "the cloud" }
            assertTrue(got.message, got.message.startsWith("Command Mode would send the selection to $where:"))
        }
        assertTrue(!(refusals.getValue("LiteLLM (10.0.0.5)") as CommandOutcome.Failed).message.contains("OpenRouter"))
        assertTrue(!CommandOutcome.NO_LLM_MESSAGE.contains("OpenRouter"))
    }

    @Test fun garbageNeverWritesText() {
        for (bad in listOf(null, "", "not json", """{"ok":true}""", """{"ok":true,"text":""}""", "{}")) {
            assertTrue(bad.toString(), CommandOutcome.decode(bad) is CommandOutcome.Failed)
        }
    }
}
