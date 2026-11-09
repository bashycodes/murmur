package dev.flowclone.data

import dev.flowclone.core.DictEntry
import dev.flowclone.learn.Suggestion
import dev.flowclone.learn.Suggestions
import dev.flowclone.notes.Note
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The backup file format (docs/backup-format.md): round trip, secrets, version and damage checks. */
class BackupTest {
    private val meta = Backup.Meta("0.7.1", "android", "hybrid", "2026-10-29T10:00:00+02:00")

    private val full = Backup.Data(
        settings = mapOf(
            "language" to Backup.Pref.Str("he"),
            "llm_enabled" to Backup.Pref.Bool(true),
            "cap_minutes" to Backup.Pref.Int32(7),
            "bubble_scale" to Backup.Pref.F32(0.9f),
            "user_denylist" to Backup.Pref.StrSet(setOf("com.bank", "com.vault")),
        ),
        developer = mapOf(
            "server.host" to Backup.Pref.Str("192.168.1.5"),
            "server.port" to Backup.Pref.Int64(8765),
            "allow_cloud" to Backup.Pref.Bool(true),
        ),
        dictionary = listOf(DictEntry("Kubernetes"), DictEntry("brb", "be right back", isSnippet = true)),
        suggestions = Suggestions(listOf(Suggestion("Zelda", "zelda", "com.chat", 5L, 2)), listOf("teh")),
        insights = Insights.Aggregates(true, mapOf("2026-10-28" to Insights.DayStats(12, 3000, 2, mapOf("com.chat" to 12L)))),
        notes = listOf(
            Note("scratchpad", "quick\nthought", true, 1L, 2L),
            Note("n1-abcd", "שלום \"quoted\"   text", false, 3L, 4L),
            Note("n2-empty", "", false, 5L, 5L),
        ),
        history = listOf(
            HistoryRow(0, 100, "com.chat", "FORMATTED", "hi there", "Hi there.", true, 1200, asrMs = 300, totalMs = 900, insertOutcome = "inserted", route = "local"),
            HistoryRow(0, 200, null, HistoryRow.COMMAND, "make it formal", "Good day.", false, 0, commandOriginal = "hey", fallbackReason = "timeout"),
        ),
    )

    private fun ok(text: String) = Backup.decode(text) as Backup.Parsed.Ok

    private fun rejected(text: String) = (Backup.decode(text) as Backup.Parsed.Rejected).reason

    @Test fun roundTripKeepsEverySection() {
        val back = ok(Backup.encode(meta, full))
        assertEquals(meta, back.meta)
        assertEquals(full, back.data)
    }

    @Test fun headerIsTheDocumentedShape() {
        val o = JSONObject(Backup.encode(meta, full))
        assertEquals("flow-clone-backup", o.getString("format"))
        assertEquals(1, o.getInt("version"))
        assertEquals("0.7.1", o.getString("app_version"))
        assertEquals(meta.created, o.getString("created"))
        val d = o.getJSONObject("data")
        for (k in listOf("settings", "developer", "dictionary", "learn_suggestions", "insights", "notes", "history")) assertTrue(k, d.has(k))
    }

    @Test fun missingSectionsStayNullSoRestoreLeavesThemAlone() {
        val back = ok(Backup.encode(meta, Backup.Data(notes = emptyList())))
        assertEquals(emptyList<Note>(), back.data.notes)
        assertNull(back.data.history)
        assertNull(back.data.settings)
        assertNull(back.data.dictionary)
    }

    /** Fails if a sealed token, a cloud API key or anything secret-shaped ever reaches the file. */
    @Test fun secretsNeverLeaveTheDevice() {
        val sealed = "c2VjcmV0aXY=:c2VjcmV0Y2lwaGVydGV4dA=="
        val leaky = full.copy(
            developer = full.developer!! + mapOf(
                "server.token_sealed" to Backup.Pref.Str(sealed),
                CloudKeyStore.PREFIX + "openrouter" to Backup.Pref.Str(sealed),
                "some_api_key" to Backup.Pref.Str("sk-or-v1-abc"),
                "new_password" to Backup.Pref.Str("hunter2"),
            ),
            settings = full.settings!! + mapOf("server_token" to Backup.Pref.Str("tok")),
        )
        val text = Backup.encode(meta, leaky)
        for (bad in listOf(sealed, "sk-or-v1-abc", "hunter2", "token_sealed", "key_sealed", "\"tok\"")) assertFalse(bad, bad in text)
        assertEquals(full, ok(text).data)
    }

    @Test fun deviceLocalModelPathsAreLeftOut() {
        val d = full.copy(settings = full.settings!! + mapOf("model_path" to Backup.Pref.Str("/data/m.bin"), "parakeet_path" to Backup.Pref.Str("/p"), "llm_model_path" to Backup.Pref.Str("/l")))
        val text = Backup.encode(meta, d)
        assertFalse("/data/m.bin" in text)
        assertEquals(full.settings, ok(text).data.settings)
    }

    @Test fun aHandMadeFileCannotPlantASecret() {
        val text = """{"format":"flow-clone-backup","version":1,"data":{"developer":{
            "server.token_sealed":{"type":"string","value":"x:y"},
            "cloud.key_sealed.openrouter":{"type":"string","value":"x:y"},
            "server.host":{"type":"string","value":"h"}},
            "settings":{"model_path":{"type":"string","value":"/evil"}}}}"""
        val d = ok(text).data
        assertEquals(mapOf("server.host" to Backup.Pref.Str("h")), d.developer)
        assertEquals(emptyMap<String, Backup.Pref>(), d.settings)
    }

    @Test fun isSecretCoversTheStoredKeyNames() {
        for (k in listOf("server.token_sealed", "cloud.key_sealed.openrouter", "cloud.key_sealed.groq", "apiKey", "api_key")) assertTrue(k, Backup.isSecret(k))
        for (k in listOf("server.host", "cloud", "allow_cloud", "language", "keep_local_warm", "cloud_usage")) assertFalse(k, Backup.isSecret(k))
    }

    @Test fun newerVersionIsRefusedWithAClearMessage() {
        val r = rejected("""{"format":"flow-clone-backup","version":2,"data":{}}""")
        assertTrue(r, "newer version" in r && "Update the app" in r)
    }

    @Test fun notABackupIsRefused() {
        assertTrue(rejected("this is not json").contains("not a Murmur backup"))
        assertTrue(rejected("""[{"word":"x"}]""").contains("not a Murmur backup")) // a dictionary export
        assertTrue(rejected("""{"format":"something-else","version":1,"data":{}}""").contains("not a Murmur backup"))
        assertTrue(rejected("""{"format":"flow-clone-backup","data":{}}""").contains("damaged"))
        assertTrue(rejected("""{"format":"flow-clone-backup","version":0,"data":{}}""").contains("damaged"))
        assertTrue(rejected("""{"format":"flow-clone-backup","version":"1","data":{}}""").contains("damaged"))
    }

    @Test fun malformedSectionsAreRefusedWholeNotHalfRead() {
        val bad = listOf(
            """{"format":"flow-clone-backup","version":1}""",
            """{"format":"flow-clone-backup","version":1,"data":{"notes":{"id":"x"}}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"notes":[{"id":"../../etc","text":"x"}]}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"notes":[{"id":"a","text":"x"},{"id":"a","text":"y"}]}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"notes":[{"id":"a"}]}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"history":[{"status":"FORMATTED"}]}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"history":["x"]}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"settings":{"language":{"type":"blob","value":1}}}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"settings":{"cap_minutes":{"type":"int","value":"many"}}}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"dictionary":{"word":"x"}}}""",
            """{"format":"flow-clone-backup","version":1,"data":{"insights":{"days":{"2026-10-28":"lots"}}}}""",
        )
        for (t in bad) assertTrue(t, rejected(t).contains("damaged"))
        // Truncated mid-write.
        val cut = Backup.encode(meta, full).let { it.substring(0, it.length / 2) }
        assertTrue(rejected(cut).isNotEmpty())
    }

    @Test fun summaryCountsWhatWillBeReplaced() {
        val s = Backup.summary(meta, full).joinToString("\n")
        for (want in listOf("0.7.1", "3 notes", "2 dictations in History", "1 dictionary word · 1 snippet", "1 learned-word suggestion", "Insights for 1 day", "Settings (8 values)")) {
            assertTrue("$want in $s", want in s)
        }
    }

    @Test fun fileNameCarriesTheDate() {
        assertEquals("flow-clone-backup-2026-10-29.json", Backup.fileName(java.time.LocalDate.of(2026, 10, 29)))
    }
}
