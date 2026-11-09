package dev.murmur.calls

import android.Manifest
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import dev.murmur.MurmurApp
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import java.util.concurrent.Executors

/**
 * D49: new Samsung call recordings, read from the phone's media index (READ_MEDIA_AUDIO).
 * Samsung saves them under `Recordings/Call/` on the internal volume (m4a, indexed as audio).
 */
class MediaStoreRecordings(private val ctx: Context) : RecordingSource {
    override fun generation(): Long = MediaStore.getGeneration(ctx, MediaStore.VOLUME_EXTERNAL_PRIMARY)

    override fun since(after: Long): List<Recording> {
        val uri = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val cols = arrayOf(
            MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.DATE_ADDED,
            MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.GENERATION_ADDED,
        )
        // Pending (still being written) files are not returned by default; empty ones are skipped.
        val sel = "${MediaStore.MediaColumns.RELATIVE_PATH} LIKE ? AND ${MediaStore.MediaColumns.GENERATION_ADDED} > ? AND ${MediaStore.MediaColumns.SIZE} > 0"
        val out = ArrayList<Recording>()
        ctx.contentResolver.query(uri, cols, sel, arrayOf("$CALL_DIR%", after.toString()), "${MediaStore.MediaColumns.GENERATION_ADDED} ASC")?.use { c ->
            while (c.moveToNext()) {
                val id = c.getLong(0)
                val modified = c.getLong(3).takeIf { it > 0 } ?: c.getLong(2)
                out += Recording(id, android.content.ContentUris.withAppendedId(uri, id).toString(), c.getString(1) ?: "recording", modified * 1000, c.getLong(4))
            }
        }
        return out
    }

    companion object {
        const val CALL_DIR = "Recordings/Call/"
    }
}

/**
 * D49 wiring: no polling. JobScheduler content-URI triggers on the audio collection wake
 * [CallRecordingJob] after a media change (also when the app is not running); while the process
 * lives a [ContentObserver] does the same sooner. Either only schedules [CallRecordingJob.RUN],
 * which scans ([AutoQueue.scan]) and hands queued recordings to [CallJobService] (or, when
 * Android refuses a foreground start from the background, transcribes inside the job). Nothing
 * holds a wake lock while idle; the opt-in lives in [AutoQueue]'s file, and [arm] runs again
 * after a reboot or an update ([dev.murmur.receiver.BootReceiver]) and at every app start.
 */
object AutoCalls {
    private const val TAG = "MurmurCalls"
    private const val PREFS = "calls"
    private const val KEY_LANG = "language"
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "murmur-calls-auto").apply { isDaemon = true } }
    private var observer: ContentObserver? = null

    fun hasPermission(ctx: Context) = ctx.checkSelfPermission(Manifest.permission.READ_MEDIA_AUDIO) == PackageManager.PERMISSION_GRANTED

    /** The language of call jobs: the Calls page's choice, else the dictation language. */
    fun language(app: MurmurApp): String = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_LANG, null) ?: CallEngine.defaultLanguage(app)

    fun setLanguage(ctx: Context, l: String) { ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY_LANG, l).apply() }

    /** Opt in (the page asked for READ_MEDIA_AUDIO first) and start watching. */
    fun enable(ctx: Context) {
        val app = MurmurApp.of(ctx)
        app.autoCalls.enable()
        arm(ctx)
    }

    fun disable(ctx: Context) {
        val app = MurmurApp.of(ctx)
        app.autoCalls.disable()
        val js = ctx.getSystemService(JobScheduler::class.java)
        for (id in CallRecordingJob.IDS) js?.cancel(id)
        observer?.let { runCatching { ctx.contentResolver.unregisterContentObserver(it) } }
        observer = null
    }

    /** Boot, update, app start, opt-in: watch again (when on) and catch up once. Cheap; never throws. */
    fun arm(ctx: Context) {
        runCatching {
            val app = MurmurApp.of(ctx)
            if (!app.autoCalls.isEnabled) return
            val js = ctx.getSystemService(JobScheduler::class.java) ?: return
            if (CallRecordingJob.TRIGGERS.none { js.getPendingJob(it) != null }) CallRecordingJob.scheduleTrigger(ctx, CallRecordingJob.TRIGGERS[0])
            CallRecordingJob.scheduleRun(ctx)
            observe(ctx.applicationContext)
        }.onFailure { Log.w(TAG, "arm: ${it.message}") }
    }

    /** The page is open (the app is in the foreground): scan and start the queue now. */
    fun kick(ctx: Context, done: (() -> Unit)? = null) {
        val app = MurmurApp.of(ctx)
        io.execute {
            runCatching { app.autoCalls.scan() }
            runCatching { startService(ctx) }
            done?.invoke()
        }
    }

    /** Hands the queue to [CallJobService]; false when there is nothing to do or Android refused. */
    fun startService(ctx: Context): Boolean {
        val app = MurmurApp.of(ctx)
        if (app.calls.state.value.busy || app.autoCalls.pending() == 0) return false
        return try {
            ctx.startForegroundService(CallJobService.drainIntent(ctx))
            true
        } catch (e: Exception) { // ForegroundServiceStartNotAllowedException from the background
            Log.i(TAG, "foreground start refused: ${e.javaClass.simpleName}")
            false
        }
    }

    @Synchronized
    private fun observe(ctx: Context) {
        if (observer != null) return
        val main = Handler(Looper.getMainLooper())
        val schedule = Runnable { CallRecordingJob.scheduleRun(ctx) }
        observer = object : ContentObserver(main) {
            override fun onChange(selfChange: Boolean, uri: Uri?) {
                // Debounced: a recording's insert and its updates arrive as several changes.
                main.removeCallbacks(schedule)
                main.postDelayed(schedule, 3_000)
            }
        }.also { ctx.contentResolver.registerContentObserver(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, true, it) }
    }
}

/**
 * D49 jobs. [TRIGGERS]: two content-URI trigger jobs that take turns (a job re-scheduling its
 * own id would stop itself), each fires once after the audio collection changes, re-arms the
 * other and schedules [RUN]. [RUN] (battery not low): scan, then start [CallJobService]; when
 * Android refuses that from the background, transcribe the queue right here, one at a time,
 * behind dictation; [onStopJob] puts the job back in the queue.
 */
class CallRecordingJob : JobService() {
    @Volatile private var stopped = false
    @Volatile private var mine: Long? = null

    override fun onStartJob(params: JobParameters): Boolean {
        val app = MurmurApp.of(this)
        if (!app.autoCalls.isEnabled) return false
        if (params.jobId in TRIGGERS) {
            val other = TRIGGERS.first { it != params.jobId }
            scheduleTrigger(this, other)
            scheduleRun(this)
            return false
        }
        Thread({
            runCatching { work(app) }.onFailure { Log.w("MurmurCalls", "run job: ${it.message}") }
            mine = null
            jobFinished(params, false)
        }, "murmur-calls-job").apply { isDaemon = true }.start()
        return true
    }

    private fun work(app: MurmurApp) {
        while (!stopped) {
            app.autoCalls.scan()
            // Busy: a manual job, or the service's queue; the service takes the next one after it.
            if (app.autoCalls.pending() == 0 || app.calls.state.value.busy) return
            if (AutoCalls.startService(this)) return
            // Background: Android refused the foreground start. Run here, inside the job's own time.
            if (!app.calls.startNext()) return
            mine = app.calls.current?.autoId
            runBlocking { app.calls.state.first { !it.busy } }
        }
    }

    override fun onStopJob(params: JobParameters): Boolean {
        stopped = true
        val app = MurmurApp.of(this)
        val cur = app.calls.current
        if (cur?.autoId != null && cur.autoId == mine) app.calls.cancel(CallStop.SYSTEM)
        return true
    }

    companion object {
        val TRIGGERS = intArrayOf(4901, 4902)
        const val RUN = 4903
        val IDS = intArrayOf(TRIGGERS[0], TRIGGERS[1], RUN)

        fun scheduleTrigger(ctx: Context, id: Int) {
            val info = JobInfo.Builder(id, ComponentName(ctx, CallRecordingJob::class.java))
                .addTriggerContentUri(JobInfo.TriggerContentUri(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, JobInfo.TriggerContentUri.FLAG_NOTIFY_FOR_DESCENDANTS))
                .setTriggerContentUpdateDelay(5_000)
                .setTriggerContentMaxDelay(60_000)
                .build()
            runCatching { ctx.getSystemService(JobScheduler::class.java)?.schedule(info) }
        }

        /** No-op while one is waiting or running (scheduling the same id would stop a running one). */
        fun scheduleRun(ctx: Context) {
            val js = ctx.getSystemService(JobScheduler::class.java) ?: return
            if (js.getPendingJob(RUN) != null) return
            val info = JobInfo.Builder(RUN, ComponentName(ctx, CallRecordingJob::class.java))
                .setRequiresBatteryNotLow(true)
                .build()
            runCatching { js.schedule(info) }
        }
    }
}
