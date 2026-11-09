package dev.murmur.probe

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.Executors

/**
 * Folder batch for the quality scorecard (hybrid debug only): every WAV under
 * `<external files>/nano-batch/<set>/` through each chosen engine, one after the other, from the
 * probe screen (GenAI needs the app on top). Each (file, engine) run appends one line
 * ([ProbeBatchPlan.line]) to `nano-batch/results/batch-<UTC start>.jsonl`, a new file per batch.
 * Clips are fed at speaking speed when the screen's real-time pacing is on. Runs here are not
 * written to probe-english.jsonl; logcat gets progress only, not the text.
 */
object ProbeBatch {
    data class Engines(val genAi: Boolean, val genAiAdvanced: Boolean, val platform: Boolean)

    private data class Job(val set: String, val file: File, val genAi: Boolean)

    private val main = Handler(Looper.getMainLooper())
    /** WAV reads and resampling, off the main thread. */
    private val io = Executors.newSingleThreadExecutor()
    @Volatile var running = false
        private set
    private var stopRequested = false

    fun root(ctx: Context): File? = ctx.getExternalFilesDir(ProbeBatchPlan.DIR)

    /** What [run] would do now, for the screen: the folder and its sets with their WAV counts. */
    fun describe(ctx: Context): String {
        val r = root(ctx) ?: return "external files dir unavailable"
        val parts = setWavs(r, "").map { (s, wavs) -> "$s (${wavs.size} WAV)" }
        return "${r.absolutePath}\nsets: ${parts.ifEmpty { listOf("none") }.joinToString(", ")}"
    }

    fun stop() { stopRequested = true; GenAiProbe.cancel(); PlatformProbe.cancel() }

    /** Runs [onlySets] (comma list; blank = all) with [engines]. [progress] and [done] on the main thread. */
    fun run(ctx: Context, onlySets: String, engines: Engines, progress: (String) -> Unit, done: (String) -> Unit) {
        if (running || GenAiProbe.running || PlatformProbe.running) { progress("a run is already going"); return }
        val app = ctx.applicationContext
        val r = root(app) ?: run { done("external files dir unavailable"); return }
        val jobs = setWavs(r, onlySets).flatMap { (s, wavs) ->
            wavs.flatMap { f -> listOfNotNull(Job(s, f, true).takeIf { engines.genAi }, Job(s, f, false).takeIf { engines.platform }) }
        }
        if (jobs.isEmpty()) { done("nothing to run: no WAVs under ${r.absolutePath}/<set>/ or no engine ticked"); return }
        val stamp = SimpleDateFormat("yyyyMMdd'T'HHmmss'Z'", Locale.US).apply { timeZone = TimeZone.getTimeZone("UTC") }.format(Date())
        val out = File(File(r, ProbeBatchPlan.RESULTS).apply { mkdirs() }, ProbeBatchPlan.resultsName(stamp))
        running = true
        stopRequested = false
        val counts = linkedMapOf("ok" to 0, "empty" to 0, "error" to 0, "cancelled" to 0)
        val t0 = ProbeRuntime.now()

        fun finish(why: String) {
            running = false
            val s = "$why · ${counts.entries.joinToString(" ") { "${it.key} ${it.value}" }} · " +
                "${(ProbeRuntime.now() - t0) / 1000} s → ${out.absolutePath}"
            Log.i(ProbeRuntime.TAG, "batch: $s")
            done(s)
        }

        fun record(set: String, id: String, rec: ProbeRecord) {
            runCatching { out.appendText(ProbeBatchPlan.line(set, id, rec) + "\n") }
                .onFailure { Log.w(ProbeRuntime.TAG, "batch: write failed", it) }
            counts[rec.outcome] = (counts[rec.outcome] ?: 0) + 1
        }

        var next: (Int) -> Unit = {}

        fun start(i: Int, j: Job, id: String, engine: String, pcm: ShortArray) {
            val clip = ProbeRuntime.Clip(pcm, "clip_batch", "${j.set}/$id")
            var settled = false
            fun settle(rec: ProbeRecord) {
                if (settled) return
                settled = true
                record(j.set, id, rec)
                Log.i(ProbeRuntime.TAG, "batch: ${j.set}/$id $engine ${rec.outcome} final+${rec.timing.latencyMs} ms")
                // Let the engine release its session before the next one starts.
                main.postDelayed({ next(i + 1) }, GAP_MS)
            }
            // No result in time: cancel the run (its record says "cancelled"); a run that never
            // reports (e.g. the GenAI client failed to start) is logged as an error and skipped.
            val giveUp = Runnable { settle(failed(engine, engines, "batch watchdog: the run never reported")) }
            val watchdog = Runnable {
                Log.w(ProbeRuntime.TAG, "batch: ${j.set}/$id $engine watchdog, cancelling")
                if (j.genAi) GenAiProbe.cancel() else PlatformProbe.cancel()
                main.postDelayed(giveUp, GIVE_UP_MS)
            }
            main.postDelayed(watchdog, ProbeBatchPlan.watchdogMs(clip.durationMs, ProbeRuntime.realTimePacing))
            val l = object : RunListener {
                override fun onState(s: String) {}
                override fun onPartial(text: String) {}
                override fun onFinal(text: String) {}
                override fun onDone(record: ProbeRecord, line: String) {
                    main.removeCallbacks(watchdog)
                    main.removeCallbacks(giveUp)
                    settle(record)
                }
            }
            if (j.genAi) GenAiProbe.run(app, engines.genAiAdvanced, clip, "batch", l, log = false)
            else PlatformProbe.run(app, clip, emptyList(), formatting = false, trigger = "batch", l = l, segmented = true, log = false)
        }

        fun step(i: Int, waitedMs: Long = 0L) {
            if (stopRequested) { finish("stopped at ${i}/${jobs.size}"); return }
            if (i >= jobs.size) { finish("done ${jobs.size}/${jobs.size}"); return }
            val j = jobs[i]
            val id = ProbeBatchPlan.clipId(j.file.name)
            val engine = if (j.genAi) "genai" else "platform"
            // A cancelled run can still be winding down; the engine refuses a new run until it is
            // idle (and would never report), so wait for it, bounded, then skip this clip.
            if (if (j.genAi) GenAiProbe.running else PlatformProbe.running) {
                if (waitedMs < BUSY_WAIT_MS) { main.postDelayed({ step(i, waitedMs + BUSY_POLL_MS) }, BUSY_POLL_MS); return }
                Log.w(ProbeRuntime.TAG, "batch: ${j.set}/$id $engine engine still busy, skipping")
                record(j.set, id, failed(engine, engines, "engine busy: the previous run had not ended after $BUSY_WAIT_MS ms"))
                main.post { step(i + 1) }
                return
            }
            progress("${i + 1}/${jobs.size} · ${j.set}/$id · $engine")
            io.execute {
                val pcm = runCatching { ProbePcm.to16kMono(ProbePcm.readWav(j.file.readBytes())) }
                main.post {
                    pcm.fold(
                        onSuccess = { start(i, j, id, engine, it) },
                        onFailure = { e ->
                            record(j.set, id, failed(engine, engines, "unreadable WAV: ${e.message}"))
                            step(i + 1)
                        },
                    )
                }
            }
        }

        next = { step(it) }
        step(0)
    }

    /** Each set folder to run (by [ProbeBatchPlan.sets]) with its WAVs in run order. */
    private fun setWavs(root: File, only: String): List<Pair<String, List<File>>> =
        ProbeBatchPlan.sets(root.listFiles()?.filter { it.isDirectory }?.map { it.name }.orEmpty(), only).map { s ->
            val dir = File(root, s)
            s to ProbeBatchPlan.wavs(dir.list()?.toList().orEmpty()).map { File(dir, it) }
        }

    private fun failed(engine: String, e: Engines, why: String) = ProbeRecord(
        tsMs = System.currentTimeMillis(), appBuild = ProbeRuntime.build(), engine = engine,
        mode = if (engine == "genai") ProbeCodes.mode(e.genAiAdvanced) else null, source = "clip_batch", trigger = "batch",
        probeOnTop = ProbeRuntime.probeOnTop, status = null, outcome = "error", text = "", partials = 0,
        timing = ProbeTiming(ProbeRuntime.now()), audioMs = null, error = why,
    )

    private const val GAP_MS = 500L
    private const val GIVE_UP_MS = 5_000L
    private const val BUSY_POLL_MS = 250L
    private const val BUSY_WAIT_MS = 10_000L
}
