package dev.flowclone.ui

import android.app.Activity
import android.content.Intent
import dev.flowclone.FlowApp
import dev.flowclone.data.HistoryRow
import dev.flowclone.data.Retention
import dev.flowclone.data.SettingsPages
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import org.robolectric.shadows.ShadowToast

/** History list (day groups, outcome badge, not-inserted recovery) and detail page. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class HistoryUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    @After fun clear() { app.history.clear() }

    private fun add(text: String, inserted: Boolean = true, status: String = "RAW_TRANSCRIPT", asr: String = text, at: Long = System.currentTimeMillis()) =
        app.history.add(HistoryRow(0, at, "com.example.chat", status, asr, text, inserted, 2_100, asrMs = 830, totalMs = 1_400), Retention.KEEP)

    /** The page loads on a thread and posts back: wait for [ready]. */
    private fun <T : Activity> settle(a: T, ready: (T) -> Boolean): T {
        repeat(200) { idle(); if (ready(a)) return a; Thread.sleep(10) }
        idle(); return a
    }

    private fun launchList() = Robolectric.buildActivity(HistoryActivity::class.java).setup().visible().get()

    @Test fun emptyHistoryShowsStatusAndEmptyState() {
        val a = settle(launchList()) { "No dictations yet" in texts(it) }
        val t = texts(a)
        for (want in listOf("STATUS", "Keep dictations", "Keep everything", "No dictations saved · audio is never stored", "Show the raw transcript", "No dictations yet")) assertTrue(want, want in t)
        rowTitled(a, "Keep dictations").performClick()
        val i: Intent = shadowOf(a).nextStartedActivity
        assertEquals(SettingsPages.Page.HISTORY.name, i.getStringExtra(SettingsActivity.EXTRA_PAGE))
    }

    @Test fun rowsAreGroupedByDayWithBadgeAndOpenTheirDetail() {
        add("Dear Sam,\nthe slides are attached.", status = "FORMATTED", asr = "dear sam the slides are attached")
        add("Yesterday's note", at = System.currentTimeMillis() - 86_400_000L)
        val a = settle(launchList()) { "Dear Sam," in texts(it) }
        val t = texts(a)
        assertTrue(t.toString(), "Today" in t)
        assertTrue("Yesterday" in t)
        assertTrue("AI cleaned" in t)
        assertTrue("2 dictations · audio is never stored" in t)
        val row = rowTitled(a, "Dear Sam,\nthe slides are attached.")
        // One TalkBack stop: the text, then when / where / how.
        assertTrue(row.contentDescription.toString(), row.contentDescription.startsWith("Dear Sam,\nthe slides are attached. ") && "com.example.chat" in row.contentDescription && row.contentDescription.endsWith("AI cleaned"))
        // The card's Copy pill copies without opening anything.
        walk(a.window.decorView).first { it.contentDescription?.startsWith("Copy dictation from") == true }.performClick()
        assertEquals("Copied (cleared after 1 minute)", ShadowToast.getTextOfLatestToast())
        assertEquals(null, shadowOf(a).nextStartedActivity)
        assertTrue(walk(a.window.decorView).any { it.contentDescription?.startsWith("More actions for dictation from") == true })
        row.performClick()
        val i = shadowOf(a).nextStartedActivity
        assertEquals(HistoryActivity::class.java.name, i.component?.className)
        assertTrue(i.getLongExtra(HistoryActivity.EXTRA_ID, -1) > 0)
        // Raw switch shows what the engine heard.
        rowTitled(a, "Show the raw transcript").performClick()
        assertTrue("dear sam the slides are attached" in texts(a))
        UiTestKit.assertTouchTargets(a, "history list")
    }

    @Test fun notInsertedRowCopiesOnTapAndKeepsDetailsReachable() {
        add("Lost text", inserted = false)
        val a = settle(launchList()) { "Lost text" in texts(it) }
        assertTrue("Not inserted — tap to copy" in texts(a))
        val row = rowTitled(a, "Lost text")
        assertTrue(row.contentDescription.toString(), row.contentDescription.endsWith("Not inserted — tap to copy"))
        row.performClick()
        assertEquals("Copied (cleared after 1 minute)", ShadowToast.getTextOfLatestToast())
        assertEquals(null, shadowOf(a).nextStartedActivity)
        // Details stay reachable: ⋮ and a TalkBack action on the card.
        val actions = row.createAccessibilityNodeInfo().actionList
        assertTrue(actions.any { it.label == "Delete" })
        row.performAccessibilityAction(actions.first { it.label == "Open details" }.id, null)
        assertEquals(HistoryActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
        UiTestKit.assertTouchTargets(a, "history list, not inserted")
    }

    @Test fun detailShowsTextTimingsAndDeletes() {
        add("Full text\nsecond line", inserted = false, asr = "full text second line")
        val id = app.history.recent().first().id
        val a = settle(Robolectric.buildActivity(HistoryActivity::class.java, HistoryActivity.detailIntent(ctx, id)).setup().visible().get()) { "DETAILS" in texts(it) }
        val t = texts(a)
        for (want in listOf("TEXT", "Full text\nsecond line", "RAW TRANSCRIPT", "full text second line", "App", "com.example.chat", "Inserted", "No", "Timings", "2.1 s audio · asr 830 ms · total 1400 ms", "Copy", "Delete dictation")) {
            assertTrue(want, want in t)
        }
        assertTrue(t.any { it.startsWith("Not inserted: this text never reached the field") })
        UiTestKit.assertTouchTargets(a, "history detail")
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Delete dictation" }.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d)
        d.getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick()
        settle(a) { it.isFinishing }
        assertTrue(a.isFinishing)
        assertTrue(app.history.recent().isEmpty())
    }

    /** The insert outcome and route that fix/route-order records show on the detail page. */
    @Test fun detailSaysWhyItWasNotInsertedAndOffersTheBubble() {
        app.history.add(
            HistoryRow(0, System.currentTimeMillis(), "com.example.chat", "RAW_TRANSCRIPT", "moved on", "moved on", false, 2_100, insertOutcome = "target_changed", route = "remote"),
            Retention.KEEP,
        )
        val id = app.history.recent().first().id
        val a = settle(Robolectric.buildActivity(HistoryActivity::class.java, HistoryActivity.detailIntent(ctx, id)).setup().visible().get()) { "DETAILS" in texts(it) }
        val t = texts(a)
        assertTrue(t.toString(), t.any { it.startsWith("Not inserted") && it.endsWith("You switched apps") })
        assertTrue(t.toString(), "Server · Transcript" in t)
        val insert = walk(a.window.decorView).first { it is android.widget.Button && it.text == HistoryActivity.REINSERT }
        UiTestKit.assertTouchTargets(a, "history detail with re-insert")
        // The service is not running in a unit test: say so instead of starting it.
        insert.performClick()
        assertEquals(HistoryActivity.REINSERT_NEEDS_SERVICE, ShadowToast.getTextOfLatestToast())
    }

    /** D68: a correction is saved as the next version; with no engine loaded the user is told its words were not learned. */
    @Test fun aCorrectionWithoutALoadedEngineSaysItsWordsWereNotLearned() {
        add("ask gina about it")
        val id = app.history.recent().first().id
        org.junit.Assume.assumeFalse(app.engine.state.value is dev.flowclone.EngineHolder.State.Ready)
        val a = settle(Robolectric.buildActivity(HistoryActivity::class.java, HistoryActivity.detailIntent(ctx, id)).setup().visible().get()) { "DETAILS" in texts(it) }
        walk(a.window.decorView).first { it is android.widget.Button && it.text == HistoryActivity.CORRECT }.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        walk(d.window!!.decorView).filterIsInstance<android.widget.EditText>().single().setText("ask Jenna about it")
        d.getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick()
        settle(a) { ShadowToast.getTextOfLatestToast() == HistoryActivity.CORRECT_NOT_LEARNED }
        assertEquals(HistoryActivity.CORRECT_NOT_LEARNED, ShadowToast.getTextOfLatestToast())
        assertEquals("ask Jenna about it", app.history.row(id)!!.finalText)
    }

    @Test fun aFailedCorrectionSaysSoInsteadOfNothingChanged() {
        val (msg, warn) = HistoryActivity.correctionMessage(IllegalStateException("disk full"), saved = false, learned = false)
        assertEquals("${HistoryActivity.CORRECT_FAILED}: disk full", msg)
        assertTrue(warn)
        assertEquals(HistoryActivity.CORRECT_UNCHANGED to false, HistoryActivity.correctionMessage(null, saved = false, learned = false))
        assertEquals(HistoryActivity.CORRECT_SAVED to false, HistoryActivity.correctionMessage(null, saved = true, learned = true))
    }

    @Test fun insertedDetailHasNoReinsert() {
        add("went in")
        val id = app.history.recent().first().id
        val a = settle(Robolectric.buildActivity(HistoryActivity::class.java, HistoryActivity.detailIntent(ctx, id)).setup().visible().get()) { "DETAILS" in texts(it) }
        assertFalse(walk(a.window.decorView).any { it is android.widget.Button && it.text == HistoryActivity.REINSERT })
    }

    @Test fun clearAllAsksFirst() {
        add("one")
        val a = settle(launchList()) { "one" in texts(it) }
        walk(a.window.decorView).first { it.contentDescription == HistoryActivity.CLEAR_ALL }.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertFalse(app.history.recent().isEmpty())
    }

    @Test fun topBarHasTheTitleAndBackFinishes() {
        val a = settle(launchList()) { "No dictations yet" in texts(it) }
        assertTrue(HistoryActivity.LIST_TITLE in texts(a))
        assertFalse("Clear all is off while there is nothing to clear", walk(a.window.decorView).first { it.contentDescription == HistoryActivity.CLEAR_ALL }.isEnabled)
        walk(a.window.decorView).first { it.contentDescription == "Back" }.performClick()
        assertTrue(a.isFinishing)
    }
}
