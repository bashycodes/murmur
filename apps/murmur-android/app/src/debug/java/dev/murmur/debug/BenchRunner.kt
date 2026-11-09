package dev.flowclone.debug

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CoreJson
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.FlowCoreLoader
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Debug builds only: the on-device benchmark loop (tools/probes/android_bench/README.md).
 *
 * Input bundle [inDir]: `manifest.json` `{"stt":[{"set","id","wav"}…],"cleanup":"cases.jsonl"}`,
 * the WAVs (16 kHz mono PCM16) and the cleanup cases (`{"id","app_id","app_category","input"}`).
 * Output [outDir]: `stt-<model>.jsonl`, `cleanup-<llm>-<mode>.jsonl`, `meta.json`, `status.json`.
 * Raw outputs only: WER and the 88-suite score are computed on Linux (`score.py`).
 *
 * STT goes through [FlowCore] exactly as a dictation does (start_session, set_context,
 * push_audio in 100 ms frames, commit); cleanup through [clean] (NativeFlowCore.benchClean: the
 * product's text path on the handle's on-device LLM). One model is loaded at a time.
 */
class BenchRunner(
    private val args: BenchArgs,
    private val filesDir: File,
    private val inDir: File,
    private val outDir: File,
    private val loader: FlowCoreLoader,
    private val clean: (FlowCore, String) -> String,
    private val device: JSONObject = JSONObject(),
    private val log: (String) -> Unit = {},
    private val stopped: () -> Boolean = { false },
    private val clockMs: () -> Long = { System.nanoTime() / 1_000_000 },
    private val sleepMs: (Long) -> Unit = Thread::sleep,
    private val extra: () -> JSONObject = { JSONObject() },
) {
    data class Clip(val set: String, val id: String, val wav: String)
    data class Case(val id: String, val appId: String, val appCategory: String, val input: String)

    private val meta = JSONObject()
    private val models = JSONArray()
    private var state = "running"
    private var phase = ""
    private var done = 0
    private var total = 0

    fun clips(): List<Clip> {
        val a = JSONObject(File(inDir, "manifest.json").readText()).optJSONArray("stt") ?: JSONArray()
        return (0 until a.length()).map { a.getJSONObject(it) }.map { Clip(it.getString("set"), it.getString("id"), it.getString("wav")) }
    }

    fun cases(): List<Case> {
        val name = JSONObject(File(inDir, "manifest.json").readText()).optString("cleanup", "")
        if (name.isEmpty()) return emptyList()
        return File(inDir, name).readLines().filter { it.isNotBlank() }.map { JSONObject(it) }.map {
            Case(it.getString("id"), it.optString("app_id", ""), it.optString("app_category", "other"), it.getString("input"))
        }.let { if (args.limit > 0) it.take(args.limit) else it }
    }

    /** Runs everything [args] asks for; never throws (errors land in meta / status). */
    fun run(): JSONObject {
        outDir.mkdirs()
        meta.put("args", JSONObject().put("suite", args.suite).put("stt", JSONArray(args.stt)).put("llm", JSONArray(args.llm))
            .put("modes", JSONArray(args.modes)).put("threads", args.threads).put("limit", args.limit)
            .put("pace", if (args.realtime) "realtime" else "fast").put("segment_pause_ms", args.segmentPauseMs ?: JSONObject.NULL))
        meta.put("device", device).put("models", models).put("started_ms", System.currentTimeMillis())
        try {
            val clips = if (args.runStt) clips() else emptyList()
            val cases = if (args.runCleanup) cases() else emptyList()
            val sttModels = if (args.runStt) BenchPlan.STT.filter { it.key in args.stt } else emptyList()
            val llms = if (args.runCleanup) args.llm else emptyList()
            total = sttModels.sumOf { m -> limited(clips.filter { it.set == m.set }).size } + llms.size * args.modes.size * cases.size
            writeStatus()
            for (m in sttModels) if (!stopped()) runStt(m, limited(clips.filter { it.set == m.set }))
            for (k in llms) if (!stopped()) runLlm(k, cases)
            state = if (stopped()) "stopped" else "done"
        } catch (e: Exception) {
            state = "error"
            meta.put("error", "${e::class.simpleName}: ${e.message}")
            log("bench error: ${e.message}")
        }
        meta.put("finished_ms", System.currentTimeMillis())
        writeJson("meta.json", meta)
        writeStatus()
        return meta
    }

    private fun limited(c: List<Clip>) = if (args.limit > 0) c.take(args.limit) else c

    private fun model(key: String, path: String): JSONObject = JSONObject().put("key", key).put("path", path).also { models.put(it) }

    private fun runStt(m: BenchPlan.Stt, clips: List<Clip>) {
        val path = File(filesDir, m.path)
        val info = model(m.key, m.path).put("kind", "stt").put("set", m.set)
        if (!path.exists()) return skip(info, "missing ${m.path}", clips.size)
        phase = "stt ${m.key}"
        writeStatus()
        val cfg = BenchPlan.modelConfig(m, filesDir, args.threads)
        val t0 = clockMs()
        val core = try {
            loader.loadModels(cfg)
        } catch (e: Exception) {
            return skip(info, "load failed: ${e.message}", clips.size)
        }
        info.put("load_ms", clockMs() - t0).put("after_load", extra())
        log("${m.key}: loaded in ${info.getLong("load_ms")} ms")
        val session = BenchPlan.sttSession(m.set, args.segmentPauseMs)
        val out = File(outDir, "stt-${m.key}.jsonl").apply { writeText("") }
        try {
            // One untimed dictation first: the app's first dictation after load pays one-off costs.
            clips.firstOrNull()?.let { info.put("warmup_ms", dictate(core, session, it).second) }
            for (c in clips) {
                if (stopped()) break
                val (row, _) = dictate(core, session, c)
                out.appendText(row.put("model", m.key).toString() + "\n")
                done++
                if (done % 5 == 0) writeStatus()
            }
            info.put("after_run", extra())
        } finally {
            core.unload()
        }
    }

    /** One dictation of [c]'s audio; (row, commit ms). */
    private fun dictate(core: FlowCore, session: dev.flowclone.core.SessionConfig, c: Clip): Pair<JSONObject, Long> {
        val row = JSONObject().put("set", c.set).put("id", c.id)
        val pcm = try {
            Wav.decode(File(inDir, c.wav).readBytes())
        } catch (e: Exception) {
            return row.put("error", "wav: ${e.message}") to 0L
        }
        row.put("audio_ms", pcm.size * 1000L / 16_000)
        try {
            core.startSession(session)
            core.setContext(FieldContext(appId = "dev.flowclone.bench", appCategory = "other"))
            val t0 = clockMs()
            var off = 0
            var refused = 0
            while (off < pcm.size) {
                val n = minOf(FRAME, pcm.size - off)
                if (args.realtime) {
                    val wait = t0 + (off + n) * 1000L / 16_000 - clockMs()
                    if (wait > 0) sleepMs(wait)
                }
                if (!core.pushAudio(pcm.copyOfRange(off, off + n), n)) refused++
                off += n
            }
            val tRelease = clockMs()
            val r = core.commit()
            val commitMs = clockMs() - tRelease
            put(row, r).put("push_ms", tRelease - t0).put("commit_ms", commitMs).put("push_refused", refused)
            return row to commitMs
        } catch (e: Exception) {
            runCatching { core.cancel() }
            return row.put("error", "${e::class.simpleName}: ${e.message}") to 0L
        }
    }

    private fun put(row: JSONObject, r: CommitResult): JSONObject = row
        .put("status", r.status.name).put("asr_text", r.asrText).put("final_text", r.historyText)
        .put("language", r.language ?: JSONObject.NULL).put("empty_reason", r.emptyReason ?: JSONObject.NULL)
        .put("error", r.error ?: JSONObject.NULL)
        .put("asr_ms", r.timings.asrMs).put("total_ms", r.timings.totalMs).put("core_audio_ms", r.timings.audioMs)
        .put("wait_ms", r.timings.waitMs).put("pre_asr_ms", r.timings.preAsrMs).put("segments", r.timings.segments)
        .put("dropped", r.timings.droppedSamples)

    private fun runLlm(key: String, cases: List<Case>) {
        val rel = BenchPlan.LLM.getValue(key)
        val path = File(filesDir, rel)
        val info = model(key, rel).put("kind", "llm")
        val n = args.modes.size * cases.size
        if (!path.isFile) return skip(info, "missing $rel", n)
        phase = "cleanup $key"
        writeStatus()
        val t0 = clockMs()
        val core = try {
            loader.loadModels(BenchPlan.llmConfig(path.path, args.threads))
        } catch (e: Exception) {
            return skip(info, "load failed: ${e.message}", n)
        }
        info.put("load_ms", clockMs() - t0).put("size_bytes", path.length()).put("after_load", extra())
        log("$key: loaded in ${info.getLong("load_ms")} ms")
        try {
            for (mode in args.modes) {
                if (stopped()) break
                phase = "cleanup $key $mode"
                val session = JSONObject(CoreJson.encode(BenchPlan.cleanupSession(mode)))
                // The app prefills the static prompt prefix at load / while the user speaks: one untimed call.
                val w = clockMs()
                clean(core, request(mode, Case("warmup", "", "other", "Warm up the cache."), session))
                info.put("warmup_${mode}_ms", clockMs() - w)
                val out = File(outDir, "cleanup-$key-$mode.jsonl").apply { writeText("") }
                for (c in cases) {
                    if (stopped()) break
                    val t = clockMs()
                    val res = runCatching { JSONObject(clean(core, request(mode, c, session))) }
                        .getOrElse { JSONObject().put("error", "bad result: ${it.message}") }
                    res.put("id", c.id).put("llm", key).put("call_ms", clockMs() - t).put("in_words", c.input.split(Regex("\\s+")).count { it.isNotEmpty() })
                    // ai-cleanup-bench --from wants `error` = the LLM's error (null when it answered).
                    if (!res.has("error")) res.put("error", res.opt("llm_error") ?: JSONObject.NULL)
                    out.appendText(res.toString() + "\n")
                    done++
                    if (done % 5 == 0) writeStatus()
                }
            }
            info.put("after_run", extra())
        } finally {
            core.unload()
        }
    }

    private fun request(mode: String, c: Case, session: JSONObject) = JSONObject()
        .put("mode", mode).put("input", c.input)
        .put("context", JSONObject(CoreJson.encode(FieldContext(appId = c.appId, appCategory = c.appCategory))))
        .put("session", session).toString()

    private fun skip(info: JSONObject, why: String, items: Int) {
        info.put("skipped", why)
        log("${info.getString("key")}: skipped ($why)")
        total -= items
        writeStatus()
    }

    private fun writeStatus() = writeJson(
        "status.json",
        JSONObject().put("state", state).put("phase", phase).put("done", done).put("total", total)
            .put("updated_ms", System.currentTimeMillis()),
    )

    private fun writeJson(name: String, o: JSONObject) {
        val tmp = File(outDir, "$name.tmp")
        tmp.writeText(o.toString(1))
        tmp.renameTo(File(outDir, name))
    }

    companion object {
        /** 100 ms, as MicRecorder / WavAudioSource deliver it. */
        const val FRAME = 1_600
    }
}
