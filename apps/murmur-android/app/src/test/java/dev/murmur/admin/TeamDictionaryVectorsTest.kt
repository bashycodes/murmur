package dev.murmur.admin

import dev.murmur.core.DictEntry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [TeamDictionary.merge] against murmur-core's team dictionary vectors
 * (crates/murmur-core/fixtures/team-dictionary/merge.json, on the test classpath; the Rust test
 * crates/murmur-core/tests/team_dictionary_vectors.rs checks `with_team` against the same file).
 */
class TeamDictionaryVectorsTest {
    private val fixture by lazy {
        JSONObject(javaClass.classLoader!!.getResource("team-dictionary/merge.json")!!.readText())
    }

    /** Raw entries: blank words and padded replacements must reach the merge as they are. */
    private fun entries(a: JSONArray) = (0 until a.length()).map { i ->
        val o = a.getJSONObject(i)
        DictEntry(
            o.getString("word"),
            if (o.has("replacement") && !o.isNull("replacement")) o.getString("replacement") else null,
            o.optBoolean("isSnippet"),
        )
    }

    @Test fun everyVectorMergesToItsWant() {
        val cases = fixture.getJSONArray("cases")
        assertTrue(cases.length() > 0)
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val got = TeamDictionary.merge(entries(c.getJSONArray("personal")), entries(c.getJSONArray("team")))
            assertEquals(c.getString("name"), entries(c.getJSONArray("want")), got)
        }
    }

    /** murmur-protocol's length vectors (team-dictionary/words.json): code points, astral ones included. */
    @Test fun wordLengthCountsCodePoints() {
        val v = JSONObject(javaClass.classLoader!!.getResource("team-dictionary/words.json")!!.readText())
        val cases = v.getJSONArray("cases")
        assertTrue(cases.length() > 0)
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            assertEquals(c.getString("name"), c.getBoolean("ok"), TeamDictionary.wordError(c.getString("word")) == null)
        }
    }

    @Test fun keyFoldsCaseAndSpacing() {
        assertEquals("acme corp", TeamDictionary.key("  Acme \t Corp "))
        assertEquals("", TeamDictionary.key("   "))
        assertEquals(TeamDictionary.key("ΟΔΟΣ"), TeamDictionary.key("οδος"))
    }
}
