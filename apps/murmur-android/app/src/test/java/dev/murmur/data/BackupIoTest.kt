package dev.flowclone.data

import android.content.Context
import dev.flowclone.FlowApp
import dev.flowclone.core.DictEntry
import dev.flowclone.learn.Suggestion
import dev.flowclone.learn.Suggestions
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Back up & restore against the app's real stores: every store out and back, secrets never out. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class BackupIoTest {
    private val app get() = RuntimeEnvironment.getApplication() as FlowApp
    private fun prefs(n: String) = app.getSharedPreferences(n, Context.MODE_PRIVATE)
    private val sealed = "aXZpdml2aXY=:Y2lwaGVydGV4dGNpcGhlcg=="

    @After fun cleanup() {
        app.history.close()
        app.deleteDatabase("history.db")
    }

    private fun seed() {
        app.settings.save(app.settings.load().copy(language = "he", capMinutes = 9, bubbleScale = 1.25f, userDenylist = setOf("com.bank")))
        app.settings.modelPath = "/data/models/ggml.bin"
        prefs(DevStore.FILE).edit()
            .putString("server.host", "10.0.0.2").putLong("server.port", 9000)
            .putString("server.token_sealed", sealed).putString(CloudKeyStore.PREFIX + "openrouter", sealed)
            .commit()
        app.dictionary.replaceAll(listOf(DictEntry("Kubernetes"), DictEntry("brb", "be right back", true)))
        app.suggestions.update { Suggestions(listOf(Suggestion("Zelda", "zelda", "com.chat", 5, 1)), listOf("teh")) }
        app.documents.save("scratchpad", "quick")
        app.documents.save("n1-test", "a note")
        app.history.add(HistoryRow(0, System.currentTimeMillis(), "com.chat", "FORMATTED", "hi", "Hi.", true, 800), Retention.KEEP)
        app.insights.record(System.currentTimeMillis(), "three little words", 900, "com.chat")
    }

    @Test fun exportHoldsEveryStoreButNoSecret() {
        seed()
        val text = BackupIo(app).export()
        for (bad in listOf(sealed, "token_sealed", "key_sealed", "/data/models/ggml.bin")) assertFalse(bad, bad in text)
        val d = (Backup.decode(text) as Backup.Parsed.Ok).data
        assertEquals(Backup.Pref.Str("he"), d.settings!!["language"])
        assertEquals(Backup.Pref.Int64(9000), d.developer!!["server.port"])
        assertEquals(2, d.dictionary!!.size)
        assertEquals("Zelda", d.suggestions!!.pending.single().word)
        assertEquals(setOf("scratchpad", "n1-test"), d.notes!!.map { it.id }.toSet())
        assertEquals("Hi.", d.history!!.single().finalText)
        assertTrue(d.insights!!.days.values.sumOf { it.words } >= 3)
    }

    @Test fun restoreReplacesDataAndKeepsThisInstallsSecretsAndModels() {
        seed()
        val insights = app.insights.load().days
        val text = BackupIo(app).export()

        // The data changes after the backup ("a version that wipes my data" or just more use).
        app.settings.save(app.settings.load().copy(language = "fr", capMinutes = 2))
        app.dictionary.replaceAll(listOf(DictEntry("other")))
        app.suggestions.update { Suggestions() }
        app.documents.delete("n1-test")
        app.documents.save("n9-new", "made later")
        app.history.clear()
        app.insights.reset()
        prefs(DevStore.FILE).edit().putString("server.host", "changed").putString("server.token_sealed", "new:token").commit()
        app.settings.modelPath = "/data/models/other.bin"

        BackupIo(app).restore((Backup.decode(text) as Backup.Parsed.Ok).data)

        val s = app.settings.load()
        assertEquals("he", s.language)
        assertEquals(9, s.capMinutes)
        assertEquals(1.25f, s.bubbleScale)
        assertEquals(1.25f, app.settings.bubbleScale.value) // the running bubble sees it
        assertEquals(setOf("com.bank"), s.userDenylist)
        assertEquals(listOf("Kubernetes", "brb"), app.dictionary.all().map { it.word })
        assertEquals("Zelda", app.suggestions.load().pending.single().word)
        assertEquals(setOf("scratchpad", "n1-test"), app.documents.list().map { it.id }.toSet())
        assertEquals("a note", app.documents.load("n1-test"))
        assertEquals(listOf("Hi."), app.history.recent().map { it.finalText })
        assertEquals(insights, app.insights.load().days)
        assertEquals("10.0.0.2", prefs(DevStore.FILE).getString("server.host", null))
        // Secrets and device-local paths are this install's, not the backup's (which has none).
        assertEquals("new:token", prefs(DevStore.FILE).getString("server.token_sealed", null))
        assertEquals(sealed, prefs(DevStore.FILE).getString(CloudKeyStore.PREFIX + "openrouter", null))
        assertEquals("/data/models/other.bin", app.settings.modelPath)
    }
}
