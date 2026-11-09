package dev.flowclone.server

import android.os.Handler
import android.os.Looper
import android.widget.LinearLayout
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.data.DevPages
import dev.flowclone.data.ModelPages
import dev.flowclone.ui.DevUi
import dev.flowclone.ui.DeveloperActivity
import dev.flowclone.ui.ServerModelsHost
import dev.flowclone.ui.ServerModelsSection
import dev.flowclone.ui.Tone
import java.io.File
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/** D41 (hybrid / remote): model screens get a "From your server" block. */
internal object ServerModels {
    const val AVAILABLE = true

    fun section(host: ServerModelsHost): ServerModelsSection? =
        if (FlowApp.of(host.activity).caps.local) ServerModelsUi(host) else null
}

/**
 * One download at a time for the whole app (like Windows' one-job slot). It outlives the screen
 * that started it (rotation, back): a screen showing its slot again picks the progress up, and a
 * finished model is waiting as "Downloaded" (Use) if no screen was there to apply it.
 */
internal object ServerDownloads {
    class Job(val modelId: String, val title: String, val screen: ModelPages.Screen) {
        val cancel = AtomicBoolean(false)
        @Volatile var done = 0L
        @Volatile var total = 0L
    }

    /** A finished job's message for its screen (shown once). */
    data class Last(val screen: ModelPages.Screen, val modelId: String, val text: String, val tone: Tone, val target: File?)

    @Volatile var job: Job? = null
        private set
    @Volatile var last: Last? = null
    private val listeners = CopyOnWriteArraySet<() -> Unit>()
    private val main = Handler(Looper.getMainLooper())

    fun listen(l: () -> Unit) { listeners += l }
    fun unlisten(l: () -> Unit) { listeners -= l }
    private fun notifyMain() = main.post { listeners.forEach { it() } }

    @Synchronized
    fun start(app: FlowApp, model: LibraryModel, plan: Plan, screen: ModelPages.Screen, serverJson: String): Boolean {
        if (job != null) return false
        val j = Job(model.id, model.title, screen)
        job = j
        last = null
        thread(name = "server-model-download") {
            var lastShown = 0L
            val outcome = ServerInstaller(NativeLibraryTransport(serverJson)).install(model, plan, { done, total ->
                j.done = done; j.total = total
                val now = System.currentTimeMillis()
                if (now - lastShown >= 500) { lastShown = now; notifyMain() }
            }, { j.cancel.get() })
            last = when (outcome) {
                is ServerInstaller.Outcome.Done -> Last(screen, model.id, "${model.title} is ready.", Tone.OK, outcome.plan.target)
                ServerInstaller.Outcome.Cancelled -> Last(screen, model.id, "Download cancelled; nothing of the unfinished file was kept.", Tone.NEUTRAL, null)
                is ServerInstaller.Outcome.Failed -> {
                    app.debugLog.event("model_download", "${model.id}: ${outcome.failure.kind}: ${outcome.failure.message}")
                    Last(screen, model.id, ServerModelPages.failed(outcome.failure, outcome.resumable), Tone.BAD, null)
                }
            }
            job = null
            notifyMain()
        }
        return true
    }
}

/** The block itself (views rebuilt by each [render]; the list and messages live here). */
internal class ServerModelsUi(private val host: ServerModelsHost) : ServerModelsSection {
    private val app = FlowApp.of(host.activity)
    private val ui: DevUi get() = host.ui
    private var models: List<LibraryModel>? = null
    private var listing = false
    private var message: Pair<String, Tone>? = null
    private var progressLine: DevUi.Result? = null
    private val listener: () -> Unit = ::onJobChange

    init { ServerDownloads.listen(listener) }

    override fun detach() = ServerDownloads.unlisten(listener)

    private fun dirs() = ModelDirs(app.engine.modelsDir(), app.engine.parakeetDir(), app.engine.hebrewDir(), app.engine.llmDir())
    private fun server() = app.dev.load().effective(app.caps)

    private fun onJobChange() {
        if (host.activity.isDestroyed) return
        val j = ServerDownloads.job
        if (j != null && j.screen == host.screen && progressLine != null) {
            progressLine?.show(ServerModelPages.progress(j.title, j.done, j.total))
            return
        }
        val l = ServerDownloads.last
        if (j == null && l != null && l.screen == host.screen) {
            ServerDownloads.last = null
            message = l.text to l.tone
            if (l.target != null) host.use(l.target) else host.refresh()
            return
        }
        host.refresh()
    }

    override fun render(parent: LinearLayout): Unit = with(ui) {
        val s = server()
        parent.sectionLabel("FROM YOUR SERVER")
        if (!s.serverConfigured) {
            parent.note("Download models from your own server instead of copying files. Set it up first.")
            parent.action("Set up your server", DevUi.Style.TONAL) { DeveloperActivity.open(host.activity, DevPages.Page.SERVER) }
            return
        }
        val list = models
        val job = ServerDownloads.job
        if (list == null) {
            parent.note("Only from your server (${s.host}), never the internet. Nothing downloads until you tap Download.")
            parent.action(if (listing) "Checking your server…" else "Show models on your server", DevUi.Style.TONAL) { if (!listing) list() }
        } else {
            val mine = list.filter { ServerSlots.screenOf(it) == host.screen }
            parent.card {
                if (mine.isEmpty()) emptyState("No models for this screen", ServerModelPages.empty(host.screen))
                for (m in mine) row(m, job)
            }
            parent.action(if (listing) "Checking your server…" else "Refresh", DevUi.Style.TONAL) { if (!listing) list() }
        }
        progressLine = parent.result().also { r ->
            if (job != null && job.screen == host.screen) r.show(ServerModelPages.progress(job.title, job.done, job.total))
            else message?.let { (t, tone) -> r.show(t, tone) }
        }
    }

    private fun LinearLayout.row(m: LibraryModel, job: ServerDownloads.Job?) = with(ui) {
        val planned = ServerSlots.plan(m, dirs(), app.caps.local)
        val plan = planned.getOrNull()
        val inUse = plan != null && plan.installed && plan.target.absolutePath == host.activePath()
        val downloading = job?.modelId == m.id
        val tone = when { downloading -> Tone.NEUTRAL; plan == null -> Tone.WARN; inUse || plan.installed -> Tone.OK; else -> Tone.NEUTRAL }
        stateRow(
            if (plan?.slot?.fileExt != null) R.drawable.ic_scr_file else R.drawable.ic_scr_folder,
            m.title, ServerModelPages.subtitle(m, planned), ServerModelPages.state(plan, inUse, downloading), tone,
        )
        val chips = buildList {
            if (downloading) add(chip("Cancel") { job?.cancel?.set(true) }.apply { contentDescription = "Cancel downloading ${m.title}" })
            else ServerModelPages.action(plan, inUse)?.let { a ->
                add(chip(a) { if (plan != null) onAction(m, plan) }.apply {
                    isEnabled = job == null; alpha = if (job == null) 1f else 0.5f
                    contentDescription = "$a ${m.title}"
                })
            }
        }
        if (chips.isNotEmpty()) chipRow(*chips.toTypedArray())
    }

    private fun list() {
        listing = true
        message = null
        host.refresh()
        val json = server().toServerConfigJson().toString()
        thread(name = "server-model-list") {
            val r = NativeLibraryTransport(json).list()
            host.activity.runOnUiThread {
                listing = false
                when (r) {
                    is ListResult.Ok -> models = r.models
                    is ListResult.Failed -> {
                        app.debugLog.event("model_list", "${r.failure.kind}: ${r.failure.message}")
                        message = ServerModelPages.listFailed(r.failure) to Tone.BAD
                    }
                }
                if (!host.activity.isDestroyed) host.refresh()
            }
        }
    }

    private fun onAction(m: LibraryModel, plan: Plan) {
        if (ServerDownloads.job != null) return
        // Use: re-verify what is on disk (no bytes when intact) and point the slot at it.
        if (plan.installed) return start(m, plan)
        val left = plan.bytes - plan.bytesOnDisk
        val dir = plan.target.parentFile ?: plan.target
        dir.mkdirs()
        if (dir.usableSpace < left + 100_000_000) {
            message = ServerModelPages.noSpace(left, dir.usableSpace) to Tone.BAD
            return host.refresh()
        }
        ui.confirm(ServerModelPages.confirmTitle(m), ServerModelPages.confirmMessage(m, plan, server().host), "Download", onYes = { start(m, plan) })
    }

    private fun start(m: LibraryModel, plan: Plan) {
        message = null
        if (!ServerDownloads.start(app, m, plan, host.screen, server().toServerConfigJson().toString())) {
            Toast.makeText(host.activity, "Another model is downloading.", Toast.LENGTH_SHORT).show()
        }
        ui.announce(if (plan.installed) "Checking ${m.title}" else "Downloading ${m.title}")
        host.refresh()
    }
}
