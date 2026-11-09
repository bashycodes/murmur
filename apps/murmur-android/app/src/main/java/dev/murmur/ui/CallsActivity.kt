package dev.murmur.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.text.format.DateFormat
import android.widget.LinearLayout
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.calls.AutoCalls
import dev.murmur.calls.AutoQueue
import dev.murmur.calls.CallEngine
import dev.murmur.calls.CallJobService
import dev.murmur.calls.CallJobState
import dev.murmur.calls.CallRequest
import dev.murmur.calls.CallTranscript
import dev.murmur.calls.CallsView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import java.util.Date
import kotlin.concurrent.thread

/**
 * Calls (D46): pick a call recording with the system picker, transcribe it on the phone in
 * Hebrew or English, keep the transcript. The list shows each transcript's date, length and
 * first line; one opens with the full text, Copy and Delete. The picked file is only read.
 * The job runs in [CallJobService]; this page shows its progress while open.
 * D49: "Transcribe new call recordings" (opt-in, READ_MEDIA_AUDIO) queues every recording made
 * after it was turned on; the section shows what waits and what failed (tap to try again).
 */
class CallsActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private lateinit var ui: DevUi
    private var detailId: String? = null
    private var items: List<CallTranscript> = emptyList()
    private var jobBox: LinearLayout? = null
    private var listBox: LinearLayout? = null
    private var autoBox: LinearLayout? = null
    private var autoToggle: DevUi.ToggleRow? = null
    private var scope: CoroutineScope? = null
    private var lastState: CallJobState? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        detailId = intent.getStringExtra(EXTRA_ID)
    }

    override fun onResume() {
        super.onResume()
        load()
        if (detailId == null) {
            scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate).also { s ->
                s.launch { app.calls.state.collect { st -> onJob(st) } }
            }
            // D49: the app is in the foreground now, so the queue can start (and new files are seen).
            if (app.autoCalls.isEnabled) AutoCalls.kick(this) { runOnUiThread { fillAuto() } }
        }
    }

    override fun onPause() {
        super.onPause()
        scope?.cancel(); scope = null
    }

    private fun load() {
        val id = detailId
        thread(name = "calls-load") {
            val all = runCatching { if (id == null) app.callsStore.list() else listOfNotNull(app.callsStore.load(id)) }.getOrDefault(emptyList())
            runOnUiThread {
                if (isFinishing || isDestroyed) return@runOnUiThread
                items = all
                if (id == null) renderList() else all.firstOrNull()?.let(::renderDetail) ?: finish()
            }
        }
    }

    // ---- list ----

    private fun renderList(): Unit = with(ui) {
        rebuild {
            topBar(TITLE, DevUi.TopNav.BACK, { finish() })
            note("Transcribe a call recording on this phone. The recording is only read: it stays where it is, unchanged. " +
                "Only the transcript is kept, here.")
            val blocked = CallEngine.unavailable(app)
            if (blocked != null) {
                banner().show(blocked, Tone.WARN)
            } else {
                sectionLabel("Language of the call")
                val langs = listOf("he", "en")
                val current = language()
                segmented(langs.map(CallsView::languageName), langs.indexOf(current).coerceAtLeast(0)) { i -> setLanguage(langs[i]) }
                action(CHOOSE, DevUi.Style.PRIMARY) { pick() }
                sectionLabel("New call recordings")
                card {
                    autoToggle = toggleRow(AUTO, AUTO_DESC, app.autoCalls.isEnabled).also { r -> r.onChange { on -> setAuto(on) } }
                }
                autoBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
            }
            jobBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
            sectionLabel("Transcripts")
            listBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
        }
        fillList()
        fillAuto()
        lastState?.let { renderJob(it) }
    }

    // ---- auto-transcribe (D49) ----

    private fun setAuto(on: Boolean) {
        if (on == app.autoCalls.isEnabled) return
        if (!on) {
            thread(name = "calls-auto") { AutoCalls.disable(this); runOnUiThread { fillAuto() } }
            return
        }
        if (AutoCalls.hasPermission(this)) enableAuto()
        else requestPermissions(arrayOf(Manifest.permission.READ_MEDIA_AUDIO), REQ_AUDIO)
    }

    private fun enableAuto() {
        thread(name = "calls-auto") {
            val ok = runCatching { AutoCalls.enable(this) }.isSuccess
            runOnUiThread {
                if (!ok) { autoToggle?.isChecked = false; toast("Couldn't turn it on") }
                fillAuto()
            }
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQ_AUDIO) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            enableAuto()
        } else {
            autoToggle?.isChecked = false
            toast("Allow Music and audio for Murmur to transcribe new call recordings")
        }
    }

    private fun fillAuto(): Unit = with(ui) {
        val box = autoBox ?: return@with
        box.removeAllViews()
        val q = app.autoCalls
        autoToggle?.isChecked = q.isEnabled
        if (!q.isEnabled) return@with
        val problem = when {
            !AutoCalls.hasPermission(this@CallsActivity) -> AutoQueue.NO_ACCESS
            else -> q.lastScanError
        }
        problem?.let { box.banner().show(it, Tone.WARN) }
        val waiting = q.pending()
        if (waiting > 0) box.note(if (waiting == 1) "1 recording waiting to be transcribed." else "$waiting recordings waiting to be transcribed.")
        val failed = q.failures()
        if (failed.isEmpty()) return@with
        box.card {
            cardHeader("Couldn't transcribe", failed.size.toString())
            for (f in failed) {
                val meta = DateFormat.getMediumDateFormat(this@CallsActivity).format(Date(f.rec.recordedAt))
                listRow(f.rec.name, meta, flag = f.error ?: "Failed", flagTone = Tone.BAD) {
                    ui.confirm("Try again?", "Transcribe ${f.rec.name} again.", "Try again", onYes = {
                        thread(name = "calls-auto") { q.retry(f.rec.id); AutoCalls.kick(this@CallsActivity) { runOnUiThread { fillAuto() } } }
                    })
                }
            }
        }
        box.action("Clear this list") {
            thread(name = "calls-auto") { for (f in failed) q.dismiss(f.rec.id); runOnUiThread { fillAuto() } }
        }
    }

    private fun fillList(): Unit = with(ui) {
        val box = listBox ?: return@with
        box.removeAllViews()
        if (items.isEmpty()) {
            box.card { emptyState("No transcripts yet", "Choose a call recording to transcribe it.") }
            return@with
        }
        box.card {
            for (t in items) {
                val title = CallsView.firstLine(t)
                listRow(title, meta(t), actions = listOf("Copy" to { copy(t) }, "Delete" to { delete(t) })) {
                    startActivity(detailIntent(this@CallsActivity, t.id))
                }
            }
        }
    }

    private fun meta(t: CallTranscript): String =
        DateFormat.getMediumDateFormat(this).format(Date(t.recordedAt)) + " · " + CallsView.clock(t.durationMs)

    private fun onJob(s: CallJobState) {
        val before = lastState
        lastState = s
        if (s is CallJobState.Done && before !is CallJobState.Done) load() // the new transcript
        if (!s.busy && before?.busy == true) fillAuto()
        renderJob(s)
    }

    private fun renderJob(s: CallJobState): Unit = with(ui) {
        val box = jobBox ?: return@with
        box.removeAllViews()
        when (s) {
            is CallJobState.Running -> box.card {
                val label = CallsView.progressLabel(s.name, s.fraction, s.loading)
                progress(s.fraction ?: 0f, label)
                action(CANCEL, DevUi.Style.DANGER) { startService(CallJobService.cancelIntent(this@CallsActivity)) }
            }
            is CallJobState.Failed -> box.banner().show("Couldn't transcribe ${s.name}: ${s.message}", Tone.BAD)
            is CallJobState.Cancelled -> box.banner().show("Transcription of ${s.name} cancelled. Nothing was saved.")
            is CallJobState.Done -> box.banner().show("Transcribed ${s.name}.", Tone.OK)
            CallJobState.Idle -> Unit
        }
    }

    private fun pick() {
        // An automatic job steps aside for a picked file (D49); a picked one doesn't.
        if (app.calls.state.value.busy && app.calls.current?.auto != true) { toast("A recording is already being transcribed"); return }
        val i = Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("audio/*")
        try {
            startActivityForResult(i, REQ_PICK)
        } catch (e: android.content.ActivityNotFoundException) {
            toast("No file picker on this phone")
        }
    }

    @Deprecated("Activity result API (the app does not use androidx.activity)")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_PICK || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        start(uri)
    }

    /** Hands the picked recording to the service (read grant included). */
    internal fun start(uri: Uri) {
        val (name, modified) = describe(uri)
        val req = CallRequest(uri.toString(), name, modified, language())
        try {
            startForegroundService(CallJobService.startIntent(this, req).setData(uri))
        } catch (e: Exception) {
            toast("Couldn't start the transcription: ${e.message}")
        }
    }

    /** Display name and last-modified time from the provider, when it has them. */
    private fun describe(uri: Uri): Pair<String, Long> {
        var name = uri.lastPathSegment ?: "recording"
        var modified = 0L
        runCatching {
            contentResolver.query(uri, null, null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }?.let { c.getString(it) }?.let { name = it }
                    c.getColumnIndex("last_modified").takeIf { it >= 0 }?.let { if (!c.isNull(it)) modified = c.getLong(it) }
                }
            }
        }
        return name to modified
    }

    private fun language(): String = AutoCalls.language(app)
    private fun setLanguage(l: String) = AutoCalls.setLanguage(this, l)

    // ---- detail ----

    private fun renderDetail(t: CallTranscript): Unit = with(ui) {
        rebuild {
            topBar(DETAIL_TITLE, DevUi.TopNav.BACK, { finish() })
            card {
                infoRow("Recorded", DateFormat.getLongDateFormat(this@CallsActivity).format(Date(t.recordedAt)) + ", " +
                    DateFormat.getTimeFormat(this@CallsActivity).format(Date(t.recordedAt)))
                infoRow("Length", CallsView.clock(t.durationMs))
                infoRow("Language", CallsView.languageName(t.language))
                infoRow("File", t.sourceName)
            }
            card { selectableText(CallsView.text(t)) }
            action(COPY, DevUi.Style.PRIMARY) { copy(t) }
            action(DELETE, DevUi.Style.DANGER) { delete(t) }
        }
    }

    // ---- actions ----

    /** S16: sensitive clip, removed again after a minute. */
    private fun copy(t: CallTranscript) {
        app.clipboard.copy(CallsView.text(t))
        toast("Copied (cleared after 1 minute)")
        ui.announce("Copied")
    }

    private fun delete(t: CallTranscript) {
        ui.confirm("Delete this transcript?", "The transcript is deleted from this phone. The recording itself is not touched.", "Delete", onYes = {
            thread(name = "calls-delete") {
                app.callsStore.delete(t.id)
                runOnUiThread {
                    ui.announce("Deleted")
                    if (detailId != null) finish() else { items = items.filterNot { it.id == t.id }; fillList() }
                }
            }
        })
    }

    private fun toast(s: String) = Toast.makeText(this, s, Toast.LENGTH_SHORT).show()

    companion object {
        const val TITLE = "Calls"
        const val DETAIL_TITLE = "Call transcript"
        const val CHOOSE = "Choose a recording"
        const val CANCEL = "Cancel"
        const val COPY = "Copy"
        const val DELETE = "Delete"
        const val EXTRA_ID = "id"
        const val REQ_PICK = 41
        const val REQ_AUDIO = 42
        const val AUTO = "Transcribe new call recordings"
        const val AUTO_DESC = "Each call recording saved after you turn this on is transcribed on this phone, one at a time and never " +
            "during a dictation. Older recordings are left alone; recordings are only read."

        fun detailIntent(c: Context, id: String): Intent = Intent(c, CallsActivity::class.java).putExtra(EXTRA_ID, id)
    }
}
