package dev.murmur.ui

import android.Manifest
import android.app.Activity
import android.content.ClipboardManager
import android.content.DialogInterface
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import dev.murmur.MurmurApp
import dev.murmur.calls.AutoQueue
import dev.murmur.calls.CallJobService
import dev.murmur.calls.CallJobState
import dev.murmur.calls.Recording
import dev.murmur.calls.RecordingSource
import dev.murmur.calls.CallJobs
import dev.murmur.calls.CallTranscript
import dev.murmur.calls.CallsView
import dev.murmur.calls.CallRequest
import dev.murmur.core.MeetingSegment
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.io.File

/** D46 Calls page: list, open, copy, delete, pick, progress + cancel. Synthetic transcripts. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class CallsUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)

    @After fun cleanup() { File(ctx.filesDir, "calls").deleteRecursively(); File(ctx.filesDir, "calls-auto.json").delete() }

    private fun <A : Activity> await(a: A, want: String): A {
        repeat(300) { idle(); if (want in texts(a)) return a; Thread.sleep(10) }
        assertTrue("$want not in ${texts(a)}", want in texts(a))
        return a
    }

    private fun seed(): CallTranscript {
        val t = CallTranscript(
            "k1", 2_000, 1_700_000_000_000, "rec-1.m4a", 125_000, "en",
            listOf(MeetingSegment(0, 8000, "Can you bring the blue folder"), MeetingSegment(8000, 15_000, "on Thursday")),
        )
        app.callsStore.save(t)
        app.callsStore.save(t.copy(id = "k0", createdAt = 1_000, segments = emptyList()))
        return t
    }

    private fun list() = Robolectric.buildActivity(CallsActivity::class.java).setup().get()

    private fun detail(id: String) =
        Robolectric.buildActivity(CallsActivity::class.java, CallsActivity.detailIntent(ctx, id)).setup().get()

    @Test fun listShowsFirstLineDateAndLength() {
        seed()
        val a = await(list(), "Can you bring the blue folder")
        val t = texts(a)
        assertTrue(t.toString(), CallsView.NO_SPEECH in t)
        assertTrue(t.toString(), t.any { it.endsWith(" · 2:05") })
        assertTrue(t.indexOf("Can you bring the blue folder") < t.indexOf(CallsView.NO_SPEECH))
        assertChooser(t)
    }

    /** On-device builds offer the picker; remote (no local engine) says why it can't. */
    private fun assertChooser(t: List<String>) {
        if (app.caps.local) assertTrue(t.toString(), CallsActivity.CHOOSE in t && "Hebrew" in t && "English" in t)
        else assertTrue(t.toString(), CallsActivity.CHOOSE !in t && t.any { it.startsWith("This build has no on-device speech engine") })
    }

    @Test fun emptyListSaysWhatToDo() {
        val a = await(list(), "No transcripts yet")
        assertChooser(texts(a))
    }

    @Test fun tappingATranscriptOpensIt() {
        seed()
        val a = await(list(), "Can you bring the blue folder")
        rowTitled(a, "Can you bring the blue folder").performClick()
        val i: Intent = shadowOf(a).nextStartedActivity
        assertEquals(CallsActivity::class.java.name, i.component?.className)
        assertEquals("k1", i.getStringExtra(CallsActivity.EXTRA_ID))
    }

    @Test fun detailShowsTheTextAndCopies() {
        seed()
        val a = await(detail("k1"), "[0:00] Can you bring the blue folder\n[0:08] on Thursday")
        assertTrue(texts(a).containsAll(listOf("rec-1.m4a", "2:05", "English", CallsActivity.DETAIL_TITLE)))
        button(a, CallsActivity.COPY).performClick(); idle()
        val clip = ctx.getSystemService(ClipboardManager::class.java).primaryClip!!
        assertEquals("[0:00] Can you bring the blue folder\n[0:08] on Thursday", clip.getItemAt(0).text.toString())
    }

    @Test fun deleteAsksThenRemovesOnlyTheTranscript() {
        seed()
        val a = await(detail("k1"), "rec-1.m4a")
        button(a, CallsActivity.DELETE).performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        d.getButton(DialogInterface.BUTTON_POSITIVE).performClick()
        repeat(300) { idle(); if (a.isFinishing) return@repeat; Thread.sleep(10) }
        assertTrue(a.isFinishing)
        assertNull(app.callsStore.load("k1"))
        assertEquals(listOf("k0"), app.callsStore.list().map { it.id })
    }

    @Test fun chooseOpensTheAudioPickerAndThePickStartsTheService() {
        org.junit.Assume.assumeTrue("remote has no on-device engine", app.caps.local)
        val a = await(list(), "No transcripts yet")
        button(a, CallsActivity.CHOOSE).performClick()
        val pick = shadowOf(a).nextStartedActivityForResult
        assertEquals(Intent.ACTION_OPEN_DOCUMENT, pick.intent.action)
        assertEquals("audio/*", pick.intent.type)
        val uri = Uri.parse("content://media.test/audio/7")
        shadowOf(a).receiveResult(pick.intent, Activity.RESULT_OK, Intent().setData(uri))
        val svc = shadowOf(RuntimeEnvironment.getApplication()).nextStartedService
        assertEquals(CallJobService::class.java.name, svc.component?.className)
        assertEquals(CallJobService.ACTION_START, svc.action)
        assertEquals(uri.toString(), svc.getStringExtra(CallJobService.EXTRA_URI))
        assertTrue(svc.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
    }

    @Test fun aRunningJobShowsProgressAndCancel() {
        val before = app.calls
        try {
            // A job that never runs: the page only sees its Running state.
            app.calls = CallJobs(app.callsStore, { error("unused") }, { error("unused") }, runner = {})
            app.calls.start(CallRequest("content://x/1", "rec-9.m4a", 0, "he"))
            val a = await(list(), CallsView.progressLabel("rec-9.m4a", null, true))
            button(a, CallsActivity.CANCEL).performClick()
            val svc = shadowOf(RuntimeEnvironment.getApplication()).nextStartedService
            assertEquals(CallJobService.ACTION_CANCEL, svc.action)
        } finally {
            app.calls = before
        }
    }

    // ---- D49: transcribe new call recordings ----

    /** A synthetic media index (no real recordings). */
    private class FakeIndex : RecordingSource {
        val recs = ArrayList<Recording>()
        fun add(name: String) = Recording(500L + recs.size, "content://media/external_primary/audio/media/${500 + recs.size}", name, 1_700_000_000_000, recs.size + 1L).also { recs += it }
        override fun generation() = recs.size.toLong()
        override fun since(after: Long) = recs.filter { it.generation > after }
    }

    private fun withQueue(block: (AutoQueue, FakeIndex) -> Unit) {
        val idx = FakeIndex()
        val q = AutoQueue(File(ctx.filesDir, "calls-auto.json"), idx)
        app.autoCalls = q
        block(q, idx)
    }

    @Test fun turningItOnAsksForAudioAccessFirst() {
        org.junit.Assume.assumeTrue("remote has no on-device engine", app.caps.local)
        withQueue { q, _ ->
            val a = await(list(), CallsActivity.AUTO)
            rowTitled(a, CallsActivity.AUTO).performClick(); idle()
            val asked = shadowOf(a).lastRequestedPermission
            assertTrue(asked.requestedPermissions.contains(Manifest.permission.READ_MEDIA_AUDIO))
            assertFalse("not on before the grant", q.isEnabled)
            shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(Manifest.permission.READ_MEDIA_AUDIO)
            a.onRequestPermissionsResult(asked.requestCode, arrayOf(Manifest.permission.READ_MEDIA_AUDIO), intArrayOf(PackageManager.PERMISSION_GRANTED))
            repeat(300) { idle(); if (q.isEnabled) return@repeat; Thread.sleep(10) }
            assertTrue(q.isEnabled)
        }
    }

    @Test fun deniedAccessLeavesItOff() {
        org.junit.Assume.assumeTrue("remote has no on-device engine", app.caps.local)
        withQueue { q, _ ->
            val a = await(list(), CallsActivity.AUTO)
            rowTitled(a, CallsActivity.AUTO).performClick(); idle()
            val asked = shadowOf(a).lastRequestedPermission
            a.onRequestPermissionsResult(asked.requestCode, arrayOf(Manifest.permission.READ_MEDIA_AUDIO), intArrayOf(PackageManager.PERMISSION_DENIED))
            idle()
            assertFalse(q.isEnabled)
        }
    }

    @Test fun failuresAreShownAndCanBeTriedAgain() {
        org.junit.Assume.assumeTrue("remote has no on-device engine", app.caps.local)
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(Manifest.permission.READ_MEDIA_AUDIO)
        withQueue { q, idx ->
            q.enable()
            val r = idx.add("fake-call.m4a")
            q.scan(); q.next("en")
            q.finished(r.id, CallJobState.Failed("fake-call.m4a", "no audio track"), null)
            val a = await(list(), "fake-call.m4a")
            assertTrue(texts(a).toString(), "no audio track" in texts(a))
            rowTitled(a, "fake-call.m4a").performClick(); idle()
            ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick()
            repeat(300) { idle(); if (q.failures().isEmpty()) return@repeat; Thread.sleep(10) }
            assertTrue(q.failures().isEmpty())
        }
    }

    private fun button(a: Activity, label: String) =
        walk(a.window.decorView).first { it is android.widget.Button && it.text.toString() == label }
}
