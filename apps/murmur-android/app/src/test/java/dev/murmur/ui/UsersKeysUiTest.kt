package dev.murmur.ui

import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.DialogInterface
import android.content.Intent
import android.view.WindowManager
import android.widget.Button
import android.widget.TextView
import dev.murmur.MurmurApp
import dev.murmur.admin.AdminApi
import dev.murmur.admin.AdminCode
import dev.murmur.admin.AdminResult
import dev.murmur.admin.CreatedKey
import dev.murmur.admin.KeyView
import dev.murmur.admin.TeamDictionary
import dev.murmur.admin.TeamDictionaryApi
import dev.murmur.admin.TeamRefresh
import dev.murmur.core.DictEntry
import dev.murmur.admin.UsersKeys
import dev.murmur.admin.WhoAmI
import dev.murmur.data.DevPages.Page
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.util.concurrent.CopyOnWriteArrayList

/**
 * D39 Users & keys on Android: the Server page shows the entry only for an admin key; the page
 * lists keys with usage incl. downloads, creates (secret once, sensitive Copy, gone on Done),
 * revokes after a confirm, and closes on 403.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class UsersKeysUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val secret = "SECRET-ui-0123456789abcdefghijklmnopqrstuvw"
    private lateinit var saved: (android.content.Context) -> AdminApi?

    private class Fake(var me: AdminResult<WhoAmI> = AdminResult.Ok(WhoAmI("k-1", "owner", true))) : AdminApi {
        val keys = CopyOnWriteArrayList(listOf(
            KeyView("k-1", "owner", admin = true, created = 1_700_000_000, lastUsed = 1_700_000_050, lastConnected = 1_700_000_100, client = "murmur-android 1.0.0",
                dictations = 12, audioMs = 120_000, downloads = 3, downloadBytes = 4_200_000, current = true),
            KeyView("k-2", "laptop", created = 1_700_000_000),
        ))
        var listError: AdminResult.Err? = null
        val revoked = CopyOnWriteArrayList<String>()
        var secret = ""
        override fun whoami() = me
        override fun listKeys(): AdminResult<List<KeyView>> = listError ?: AdminResult.Ok(keys.toList())
        override fun createKey(name: String, admin: Boolean): AdminResult<CreatedKey> {
            val k = KeyView("k-${keys.size + 1}", name, admin = admin, created = 1_700_000_200)
            keys += k
            return AdminResult.Ok(CreatedKey(k, secret))
        }
        override fun revokeKey(id: String): AdminResult<KeyView> {
            revoked += id
            val i = keys.indexOfFirst { it.id == id }
            keys[i] = keys[i].copy(revoked = 1_700_000_300)
            return AdminResult.Ok(keys[i])
        }
    }

    /** The server's team dictionary (no natives). */
    private class Team(var entries: List<DictEntry> = emptyList()) : TeamDictionaryApi {
        var editError: AdminResult.Err? = null
        override fun cached(): AdminResult<List<DictEntry>> = AdminResult.Ok(entries)
        override fun refresh(): AdminResult<TeamRefresh> = AdminResult.Ok(TeamRefresh(entries, false))
        override fun add(word: String, replacement: String): AdminResult<List<DictEntry>> =
            editError ?: AdminResult.Ok((entries + DictEntry(word, replacement.ifEmpty { null }))).also { entries = it.value }
        override fun remove(word: String): AdminResult<List<DictEntry>> =
            editError ?: AdminResult.Ok(entries.filterNot { it.word == word }).also { entries = it.value }
    }

    private lateinit var savedTeam: (android.content.Context) -> TeamDictionaryApi?

    @Before fun save() { saved = UsersKeys.factory; savedTeam = TeamDictionary.factory }
    @After fun restore() {
        UsersKeys.factory = saved
        TeamDictionary.factory = { null }
        MurmurApp.of(ctx).team.refreshNow()
        TeamDictionary.factory = savedTeam
    }

    private fun use(f: Fake?): Fake? { UsersKeys.factory = { f }; return f }

    /** The page's requests run on worker threads: idle the main looper until [cond]. */
    private fun waitFor(what: String, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 5_000
        while (!cond()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            Thread.sleep(10); idle()
        }
    }

    private fun launch() = Robolectric.buildActivity(UsersKeysActivity::class.java, Intent(ctx, UsersKeysActivity::class.java)).setup().visible().get()

    private fun button(a: android.app.Activity, text: String) =
        walk(a.window.decorView).filterIsInstance<Button>().first { it.text.toString() == text && UiTestKit.visible(it) }

    @Test fun listsKeysWithUsageAndDownloads() {
        use(Fake())
        val a = launch()
        waitFor("keys") { "laptop" in texts(a) }
        val t = texts(a)
        assertTrue(t.toString(), "owner" in t && "admin · this app" in t)
        assertTrue(t.toString(), t.any { it.contains("12 dictations") && it.contains("3 downloads (4.2 MB)") })
        assertTrue(t.toString(), t.any { it.startsWith("Last admin key") })
        // Only the laptop can be revoked (the owner is the last admin key).
        assertEquals(1, walk(a.window.decorView).filterIsInstance<Button>().count { it.text.toString() == "Revoke" && UiTestKit.visible(it) })
        UiTestKit.assertTouchTargets(a, "users-keys")
    }

    @Test fun createShowsTheSecretOnceWithASensitiveCopy() {
        val f = use(Fake())!!.also { it.secret = secret }
        val a = launch()
        waitFor("keys") { "laptop" in texts(a) }
        val name = walk(a.window.decorView).filterIsInstance<android.widget.EditText>().first()
        name.setText("phone")
        button(a, "Create key").performClick()
        waitFor("dialog") { a.secretDialog != null }
        val d = a.secretDialog!!
        assertTrue(d.window!!.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        val dlgTexts = walk(d.window!!.decorView).filterIsInstance<TextView>().map { it.text.toString() }
        assertTrue(secret in dlgTexts)
        walk(d.window!!.decorView).filterIsInstance<Button>().first { it.text.toString() == "Copy" }.performClick()
        val clip = ctx.getSystemService(ClipboardManager::class.java).primaryClip!!
        assertEquals(secret, clip.getItemAt(0).text.toString())
        assertTrue(clip.description.extras.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE))
        assertFalse(clip.description.label.toString().contains(secret))
        val secretView = walk(d.window!!.decorView).filterIsInstance<TextView>().first { it.text.toString() == secret }
        d.getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertNull(a.secretDialog)
        assertEquals("", secretView.text.toString())
        waitFor("new row") { "phone" in texts(a) }
        assertFalse(texts(a).any { it.contains(secret) })
        assertEquals("phone", f.keys.last().name)
    }

    @Test fun revokeAsksFirst() {
        val f = use(Fake())!!
        val a = launch()
        waitFor("keys") { "laptop" in texts(a) }
        button(a, "Revoke").performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertEquals("Revoke laptop?", org.robolectric.Shadows.shadowOf(d).title.toString())
        d.getButton(DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertTrue(f.revoked.isEmpty())
        button(a, "Revoke").performClick(); idle()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick()
        waitFor("revoked") { "Revoked" in texts(a) }
        assertEquals(listOf("k-2"), f.revoked.toList())
    }

    @Test fun teamDictionaryListsAddsAndRemoves() {
        use(Fake())
        val team = Team(listOf(DictEntry("Quillon")))
        TeamDictionary.factory = { team }
        val a = launch()
        waitFor("team") { "Quillon" in texts(a) }
        assertTrue("TEAM DICTIONARY" in texts(a).map { it.uppercase() })
        val fields = walk(a.window.decorView).filterIsInstance<android.widget.EditText>()
        // Name, then the team word and its replacement.
        fields[1].setText("brixwell"); fields[2].setText("Brixwell")
        button(a, "Add to team dictionary").performClick()
        waitFor("added") { "Added brixwell" in texts(a) }
        assertTrue(texts(a).toString(), "Replaced with Brixwell" in texts(a))
        assertEquals(listOf(DictEntry("Quillon"), DictEntry("brixwell", "Brixwell")), MurmurApp.of(ctx).team.entries)
        assertEquals("", fields[1].text.toString())
        walk(a.window.decorView).filterIsInstance<Button>().first { it.contentDescription == "Remove Quillon" }.performClick(); idle()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick()
        waitFor("removed") { "Removed Quillon" in texts(a) }
        assertEquals(listOf(DictEntry("brixwell", "Brixwell")), team.entries)
        UiTestKit.assertTouchTargets(a, "users-keys team")
    }

    @Test fun teamDictionaryShowsTheServersReason() {
        use(Fake())
        val team = Team().also { it.editError = AdminResult.Err(TeamDictionary.TOO_MANY, "the team dictionary is full (500 entries)") }
        TeamDictionary.factory = { team }
        val a = launch()
        waitFor("empty team") { "No team words yet" in texts(a) }
        val fields = walk(a.window.decorView).filterIsInstance<android.widget.EditText>()
        button(a, "Add to team dictionary").performClick(); idle()
        assertTrue(texts(a).toString(), "Type the word." in texts(a))
        fields[1].setText("Zephyr")
        button(a, "Add to team dictionary").performClick()
        waitFor("error") { "The team dictionary is full (500 entries)" in texts(a) }
        assertFalse(a.isFinishing)
        team.editError = AdminResult.Err(AdminCode.FORBIDDEN, "admin key required")
        button(a, "Add to team dictionary").performClick()
        waitFor("forbidden") { "Only an admin key can change the team dictionary." in texts(a) }
    }

    @Test fun forbiddenClosesThePage() {
        use(Fake().also { it.listError = AdminResult.Err(AdminCode.FORBIDDEN, "admin key required") })
        val a = launch()
        waitFor("finish") { a.isFinishing }
    }

    @Test fun noClientNoPage() {
        use(null)
        assertTrue(launch().isFinishing)
    }

    private fun serverPage() = Robolectric.buildActivity(
        DeveloperActivity::class.java, Intent(ctx, DeveloperActivity::class.java).putExtra(DeveloperActivity.EXTRA_PAGE, Page.SERVER.name),
    ).setup().visible().get()

    @Test fun serverPageShowsTheEntryOnlyForAnAdminKey() {
        use(Fake())
        val a = serverPage()
        waitFor("entry") { UsersKeys.TITLE in texts(a) }
        assertTrue("USERS & KEYS" in texts(a) || "Users & keys".uppercase() in texts(a).map { it.uppercase() })
        UiTestKit.rowTitled(a, UsersKeys.TITLE).performClick(); idle()
        assertEquals(UsersKeysActivity::class.java.name, org.robolectric.Shadows.shadowOf(a).nextStartedActivity.component?.className)

        use(Fake(me = AdminResult.Err(AdminCode.FORBIDDEN)))
        val b = serverPage()
        Thread.sleep(200); idle()
        assertFalse(UsersKeys.TITLE in texts(b))

        use(Fake(me = AdminResult.Ok(WhoAmI("k-2", "phone", false))))
        val c = serverPage()
        Thread.sleep(200); idle()
        assertFalse(UsersKeys.TITLE in texts(c))

        use(null)
        assertFalse(UsersKeys.TITLE in texts(serverPage()))
    }
}
