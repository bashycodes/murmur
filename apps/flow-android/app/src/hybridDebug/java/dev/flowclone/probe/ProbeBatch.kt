package dev.flowclone.probe

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * Folder batch for the parity scorecard (hybrid debug only): every WAV under
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
    @Volatile var running = false
        private set
    private var stopRequested = false

    fun root(ctx: Context): File? = ctx.getExternalFilesDir(ProbeBatchPlan.DIR)

    /** What [run] would do now, for the screen: the folder and its sets with their WAV counts. */
    fun describe(ctx: Context): String {
        val r = root(ctx) ?: return "external files dir unavailable"
        val sets = ProbeBatchPlan.sets(r.listFiles()?.filter { it.isDirectory }?.map { it.name }.orEmpty(), "")
        val parts = sets.map { s -> "$s (${ProbeBatchPlan.wavs(File(r, s).list()?.toList().orEmpty()).size} WAV)" }
        return "${r.absolutePath}\nsets: ${parts.ifEmpty { listOf("none") }.joinToString(", ")}"
    }

    fun stop() { stopRequested = true; GenAiProbe.cancel(); PlatformProbe.cancel() }

    /** Runs [onlySets] (comma list; blank = all) with [engines]. [progress] and [done] on the main thread. */
    fun run(ctx: Context, onlySets: String, engines: Engines, progress: (String) -> Unit, done: (String) -> Unit) {
        if (running || GenAiProbe.running || PlatformProbe.running) { progress("a run is already going"); return }
        val app = ctx.applicationContext
        val r = root(app) ?: run { done("external files dir unavailable"); return }
        val sets = ProbeBatchPlan.sets(r.listFiles()?.filter { it.isDirectory }?.map { it.name }.orEmpty(), onlySets)
        val jobs = sets.flatMap { s ->
            ProbeBatchPlan.wavs(File(r, s).list()?.toList().orEmpty()).flatMap { f ->
                listOfNotNull(Job(s, File(File(r, s), f), true).takeIf { engines.genAi }, Job(s, File(File(r, s), f), false).takeIf { engines.platform })
            }
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

        fun step(i: Int) {
            if (stopRequested) { finish("stopped at ${i}/${jobs.size}"); return }
            if (i >= jobs.size) { finish("done ${jobs.size}/${jobs.size}"); return }
            val j = jobs[i]
            val id = ProbeBatchPlan.clipId(j.file.name)
            val engine = if (j.genAi) "genai" else "platform"
            progress("${i + 1}/${jobs.size} · ${j.set}/$id · $engine")
            val pcm = runCatching { ProbePcm.to16kMono(ProbePcm.readWav(j.file.readBytes())) }.getOrElse { e ->
                val rec = failed(engine, engines, "unreadable WAV: ${e.message}")
                out.appendText(ProbeBatchPlan.line(j.set, id, rec) + "\n")
                counts["error"] = counts.getValue("error") + 1
                main.post { step(i + 1) }
                return
            }
            val clip = ProbeRuntime.Clip(pcm, "clip_batch", "${j.set}/$id")
            var settled = false
            fun settle(record: ProbeRecord) {
                if (settled) return
                settled = true
                runCatching { out.appendText(ProbeBatchPlan.line(j.set, id, record) + "\n") }
                    .onFailure { Log.w(ProbeRuntime.TAG, "batch: write failed", it) }
                counts[record.outcome] = (counts[record.outcome] ?: 0) + 1
                Log.i(ProbeRuntime.TAG, "batch: ${j.set}/$id $engine ${record.outcome} final+${record.timing.latencyMs} ms")
                // Let the engine release its session before the next one starts.
                main.postDelayed({ step(i + 1) }, GAP_MS)
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
        step(0)
    }

    private fun failed(engine: String, e: Engines, why: String) = ProbeRecord(
        tsMs = System.currentTimeMillis(), appBuild = ProbeRuntime.build(), engine = engine,
        mode = if (engine == "genai") ProbeCodes.mode(e.genAiAdvanced) else null, source = "clip_batch", trigger = "batch",
        probeOnTop = ProbeRuntime.probeOnTop, status = null, outcome = "error", text = "", partials = 0,
        timing = ProbeTiming(ProbeRuntime.now()), audioMs = null, error = why,
    )

    private const val GAP_MS = 500L
    private const val GIVE_UP_MS = 5_000L
}
