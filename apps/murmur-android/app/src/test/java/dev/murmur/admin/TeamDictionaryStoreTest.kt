package dev.murmur.admin

import dev.murmur.core.DictEntry
import dev.murmur.data.BuildCaps
import dev.murmur.data.DevSettings
import dev.murmur.data.Settings
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** The in-memory team list: cache at start, refresh, offline, admin edits, merge into the session. */
class TeamDictionaryStoreTest {
    private class Fake(
        var cache: List<DictEntry> = emptyList(),
        var answer: AdminResult<TeamRefresh> = AdminResult.Ok(TeamRefresh(emptyList(), false)),
    ) : TeamDictionaryApi {
        var refreshes = 0
        var edit: AdminResult<List<DictEntry>>? = null
        val calls = mutableListOf<String>()
        override fun cached(): AdminResult<List<DictEntry>> = AdminResult.Ok(cache)
        override fun refresh(): AdminResult<TeamRefresh> { refreshes++; return answer }
        override fun add(word: String, replacement: String): AdminResult<List<DictEntry>> {
            calls += "add:$word:$replacement"
            return edit ?: AdminResult.Ok(cache + DictEntry(word, replacement.ifEmpty { null }))
        }
        override fun remove(word: String): AdminResult<List<DictEntry>> {
            calls += "remove:$word"
            return edit ?: AdminResult.Ok(cache.filterNot { it.word == word })
        }
    }

    private val quillon = DictEntry("Quillon")
    private val brixwell = DictEntry("brixwell", "Brixwell")

    @Test fun noServerIsEmptyAndFetchesNothing() {
        var asked = 0
        val s = TeamDictionaryStore { asked++; null }
        s.load()
        assertNull(s.refreshNow())
        assertEquals(emptyList<DictEntry>(), s.entries)
        assertTrue(s.add("Velloria", "") is AdminResult.Err)
        assertEquals(3, asked)
        // No network build, no address or no key (as on Windows): no client at all.
        val key = "test-token-0123456789"
        assertNull(TeamDictionary.clientFor(BuildCaps.LOCAL, DevSettings(allowServer = true, allowClient = true, host = "example.test", token = key), File("x")))
        assertNull(TeamDictionary.clientFor(BuildCaps.ALL, DevSettings(allowServer = true, allowClient = true, host = "", token = key), File("x")))
        assertNull(TeamDictionary.clientFor(BuildCaps.ALL, DevSettings(allowServer = true, allowClient = true, host = "example.test"), File("x")))
        assertTrue(TeamDictionary.clientFor(BuildCaps.ALL, DevSettings(allowServer = true, allowClient = true, host = "example.test", token = key), File("x")) is NativeTeamDictionaryApi)
    }

    @Test fun loadReadsTheCacheAndRefreshAppliesTheAnswer() {
        val f = Fake(cache = listOf(quillon))
        var changes = 0
        val s = TeamDictionaryStore { f }.also { it.onChange = { changes++ } }
        s.load()
        assertEquals(listOf(quillon), s.entries)
        assertEquals(0, f.refreshes)
        f.answer = AdminResult.Ok(TeamRefresh(listOf(quillon, brixwell), changed = true))
        assertEquals(true, s.refreshNow()?.changed)
        assertEquals(listOf(quillon, brixwell), s.entries)
        assertEquals(2, changes)
        // The same list again: no change event.
        s.refreshNow()
        assertEquals(2, changes)
    }

    @Test fun offlineKeepsTheCachedListTheNativeAnswers() {
        val f = Fake(answer = AdminResult.Ok(TeamRefresh(listOf(quillon), false, AdminResult.Err(AdminCode.CONNECT_FAILED, "offline"))))
        val s = TeamDictionaryStore { f }
        val r = s.refreshNow()!!
        assertEquals(AdminCode.CONNECT_FAILED, r.error?.code)
        assertEquals(listOf(quillon), s.entries)
        // A bad config / a build without natives keeps what the store has.
        f.answer = AdminResult.Err(AdminCode.NOT_IN_BUILD)
        assertEquals(AdminCode.NOT_IN_BUILD, s.refreshNow()?.error?.code)
        assertEquals(listOf(quillon), s.entries)
    }

    @Test fun serverGoneEmptiesTheList() {
        var f: Fake? = Fake(answer = AdminResult.Ok(TeamRefresh(listOf(quillon), true)))
        val s = TeamDictionaryStore { f }
        s.refreshNow()
        assertEquals(listOf(quillon), s.entries)
        f = null
        s.refreshNow()
        assertEquals(emptyList<DictEntry>(), s.entries)
    }

    @Test fun adminEditsReplaceTheListAndFailuresKeepIt() {
        val f = Fake(cache = listOf(quillon))
        val s = TeamDictionaryStore { f }.also { it.load() }
        var heard = 0
        val l: () -> Unit = { heard++ }
        s.addListener(l)
        assertTrue(s.add(" Velloria ", " ") is AdminResult.Ok)
        assertEquals("add:Velloria:", f.calls.last())
        assertEquals(listOf(quillon, DictEntry("Velloria")), s.entries)
        f.edit = AdminResult.Err(TeamDictionary.TOO_MANY, "the team dictionary is full (500 entries)")
        val e = s.add("Zephyr", "") as AdminResult.Err
        assertEquals("The team dictionary is full (500 entries)", TeamDictionary.errorText(e))
        assertEquals(2, s.entries.size)
        f.edit = null
        f.cache = s.entries
        s.remove("Quillon")
        assertEquals(listOf(DictEntry("Velloria")), s.entries)
        assertEquals(2, heard)
        s.removeListener(l)
        f.cache = emptyList(); s.remove("Velloria")
        assertEquals(2, heard)
    }

    @Test fun personalWinsInTheSessionConfig() {
        val f = Fake(answer = AdminResult.Ok(TeamRefresh(listOf(DictEntry("ZEPHYR", "ZephyrCo"), quillon, DictEntry("snip", "x", isSnippet = true)), true)))
        val s = TeamDictionaryStore { f }.also { it.refreshNow() }
        val personal = listOf(DictEntry("zephyr", "Zephyr"))
        val cfg = Settings().toSessionConfig(s.merged(personal))
        assertEquals(listOf(DictEntry("zephyr", "Zephyr"), quillon), cfg.dictionary)
        // D68: team words alone make hotwords.
        assertTrue(dev.murmur.data.EnginePlan.hasHotwords(s.merged(emptyList())))
        assertFalse(dev.murmur.data.EnginePlan.hasHotwords(TeamDictionaryStore { null }.merged(emptyList())))
    }

    @Test fun decodesTheNativeAnswers() {
        val r = AdminJson.decode(
            """{"ok":true,"value":{"entries":[{"word":"Quillon"},{"word":"brixwell","replacement":"Brixwell"},{"word":" "}],"changed":false,"error":{"code":"connect_failed","message":"no route"}}}""",
            TeamDictionary::refreshed,
        ) as AdminResult.Ok
        assertEquals(listOf(quillon, brixwell), r.value.entries)
        assertFalse(r.value.changed)
        assertEquals("connect_failed", r.value.error?.code)
        val fresh = AdminJson.decode("""{"ok":true,"value":{"entries":[],"changed":true}}""", TeamDictionary::refreshed) as AdminResult.Ok
        assertNull(fresh.value.error)
        val bad = AdminJson.decode("""{"ok":false,"code":"bad_request","message":"the word is too long"}""", TeamDictionary::entries) as AdminResult.Err
        assertEquals("The word is too long", TeamDictionary.errorText(bad))
        assertEquals("Only an admin key can change the team dictionary.", TeamDictionary.errorText(AdminResult.Err(AdminCode.FORBIDDEN)))
        assertEquals(null, JSONObject("{}").let(TeamDictionary::entries).firstOrNull())
    }
}
