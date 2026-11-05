package dev.flowclone.data

import org.json.JSONException
import org.json.JSONObject

/**
 * Load 3: the logcat / debug-log line for a model load (`engine_load_timings`): the core's
 * per-model, per-phase timings (`nativeLastLoadTimings`) plus what the phone measurement needs
 * to explain them (docs/findings/2026-11-05-phone-model-load-time.md): the load set, the thread
 * count the models got, and the CPUs the loading thread may run on (a background cpuset limited
 * to the little cores would explain a slow load).
 */
object LoadTimingsLog {
    fun line(timingsJson: String, set: LoadSet, threads: Int, cpus: String?): String {
        val o = try { JSONObject(timingsJson) } catch (e: JSONException) { JSONObject().put("timings", timingsJson) }
        o.put("load_set", set.toString())
        o.put("threads", threads)
        cpus?.let { o.put("cpus", it) }
        return o.toString()
    }

    /** `Cpus_allowed_list` of a `/proc/…/status` text, e.g. `0-7`; null when absent. */
    fun cpusAllowed(status: String): String? =
        status.lineSequence().firstOrNull { it.startsWith("Cpus_allowed_list:") }
            ?.substringAfter(':')?.trim()?.takeIf { it.isNotEmpty() }
}
