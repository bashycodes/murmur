package dev.flowclone.core

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [CommandOutcome.decode] against flow-core's Command Mode wire vectors
 * (crates/flow-core/fixtures/command/command_results.json, on the test classpath; the Rust
 * test crates/flow-core/tests/command_vectors.rs checks the encoder against the same file).
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

    @Test fun garbageNeverWritesText() {
        for (bad in listOf(null, "", "not json", """{"ok":true}""", """{"ok":true,"text":""}""", "{}")) {
            assertTrue(bad.toString(), CommandOutcome.decode(bad) is CommandOutcome.Failed)
        }
    }
}
