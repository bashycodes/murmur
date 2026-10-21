package dev.flowclone.core

import dev.flowclone.core.CoreJson.optLongOrNull
import dev.flowclone.core.CoreJson.optStringOrNull
import org.json.JSONException
import org.json.JSONObject

/** flow_router `Capabilities` (`nativeCapabilities()`): what this native library can do. */
data class Capabilities(
    val local: Boolean = false,
    val remote: Boolean = false,
    val asrEngines: List<String> = emptyList(),
    val llm: Boolean = false,
    val proto: Int = 0,
    val coreVersion: String = "",
)

data class Rtt(val min: Long, val median: Long, val max: Long)

/** flow_router `HealthReport` (`nativeProbeServer`, `RouteStatus.health`). */
data class HealthReport(
    val ok: Boolean,
    val engine: String = "remote",
    /** A `route.fallback` string (`connect_failed`, `auth_failed`, `proto_mismatch`, …) when not ok. */
    val error: String? = null,
    val message: String? = null,
    val server: String? = null,
    val rttMs: Rtt? = null,
    val serverVersion: String? = null,
    val serverCore: String? = null,
    val postprocessRev: String? = null,
    val postprocessMatch: Boolean? = null,
    val proto: Int? = null,
    val asrBackend: String? = null,
    val asrModel: String? = null,
    val llm: Boolean? = null,
    val workers: Int? = null,
    val busy: Int? = null,
    val checkedAtMs: Long? = null,
)

data class CircuitStatus(
    val open: Boolean = false,
    val reason: String? = null,
    val retryInMs: Long? = null,
    /** auth_failed / proto_mismatch: stays open until the server config changes. */
    val untilConfigChange: Boolean = false,
)

/** flow_router `RouteStatus` (`nativeRouteStatus`). */
data class RouteStatus(val last: RouteInfo? = null, val circuit: CircuitStatus = CircuitStatus(), val health: HealthReport? = null)

/** Router JSON (D17 §1.2) ↔ Kotlin. Pinned by RouterJsonTest against crates/flow-router/fixtures. */
object RouterJson {
    private fun JSONObject.optIntOrNull(k: String): Int? = if (!has(k) || isNull(k)) null else optInt(k)
    private fun JSONObject.optBoolOrNull(k: String): Boolean? = if (!has(k) || isNull(k)) null else optBoolean(k)

    fun decodeCapabilities(json: String): Capabilities = try {
        val o = JSONObject(json)
        val a = o.optJSONArray("asr_engines")
        Capabilities(
            local = o.optBoolean("local"), remote = o.optBoolean("remote"),
            asrEngines = if (a == null) emptyList() else (0 until a.length()).map { a.getString(it) },
            llm = o.optBoolean("llm"), proto = o.optInt("proto"), coreVersion = o.optString("core_version"),
        )
    } catch (e: JSONException) {
        Capabilities()
    }

    fun decodeHealth(json: String): HealthReport = try {
        decodeHealth(JSONObject(json))
    } catch (e: JSONException) {
        HealthReport(ok = false, error = "error", message = "bad health json: ${e.message}")
    }

    fun decodeHealth(o: JSONObject) = HealthReport(
        ok = o.optBoolean("ok"),
        engine = o.optString("engine", "remote"),
        error = o.optStringOrNull("error"),
        message = o.optStringOrNull("message"),
        server = o.optStringOrNull("server"),
        rttMs = o.optJSONObject("rtt_ms")?.let { Rtt(it.optLong("min"), it.optLong("median"), it.optLong("max")) },
        serverVersion = o.optStringOrNull("server_version"),
        serverCore = o.optStringOrNull("server_core"),
        postprocessRev = o.optStringOrNull("postprocess_rev"),
        postprocessMatch = o.optBoolOrNull("postprocess_match"),
        proto = o.optIntOrNull("proto"),
        asrBackend = o.optStringOrNull("asr_backend"),
        asrModel = o.optStringOrNull("asr_model"),
        llm = o.optBoolOrNull("llm"),
        workers = o.optIntOrNull("workers"),
        busy = o.optIntOrNull("busy"),
        checkedAtMs = o.optLongOrNull("checked_at_ms"),
    )

    fun decodeRouteStatus(json: String): RouteStatus = try {
        val o = JSONObject(json)
        val c = o.optJSONObject("circuit") ?: JSONObject()
        RouteStatus(
            last = o.optJSONObject("last")?.let(CoreJson::decodeRoute),
            circuit = CircuitStatus(
                open = c.optBoolean("open"), reason = c.optStringOrNull("reason"),
                retryInMs = c.optLongOrNull("retry_in_ms"),
                untilConfigChange = c.optBoolean("until_config_change"),
            ),
            health = o.optJSONObject("health")?.let(::decodeHealth),
        )
    } catch (e: JSONException) {
        RouteStatus()
    }
}

/** User-facing lines for the Developer page (§5), kept pure for the JVM tests. */
object DevText {
    fun probe(h: HealthReport): String {
        if (h.ok) {
            val rtt = h.rttMs?.let { "median ${it.median} ms (${it.min}-${it.max})" } ?: "no RTT"
            return listOfNotNull("OK", rtt, h.serverVersion, h.asrBackend).joinToString(" · ")
        }
        val hint = when (h.error) {
            "auth_failed" -> "token rejected"
            "connect_failed" -> "is flow-server running / Tailscale up?"
            "proto_mismatch" -> "client and server protocol versions differ (server proto ${h.proto ?: "?"})"
            "server_not_configured" -> "set the server host first"
            "busy" -> "server busy, try again"
            "timeout" -> "server too slow to answer"
            else -> null
        }
        return listOfNotNull("${h.error ?: "error"}${hint?.let { ": $it" } ?: ""}", h.message).joinToString(" — ")
    }

    fun serverInfo(h: HealthReport?): String {
        if (h == null) return "No probe yet. Save or tap Test connection."
        if (!h.ok) return "Last probe failed: ${probe(h)}"
        val pp = when (h.postprocessMatch) {
            true -> "pipeline ✓ (${h.postprocessRev})"
            false -> "⚠ server pipeline ≠ client pipeline; results may differ (${h.postprocessRev})"
            null -> "pipeline ?"
        }
        return listOf(
            "${h.serverVersion ?: "flow-server"} · core ${h.serverCore ?: "?"} · proto ${h.proto ?: "?"}",
            pp,
            "ASR ${h.asrBackend ?: "?"}${h.asrModel?.let { " ($it)" } ?: ""} · LLM ${if (h.llm == true) "on" else "off"} · workers ${h.workers ?: "?"} busy ${h.busy ?: "?"}",
        ).joinToString("\n")
    }

    fun circuit(c: CircuitStatus, nowMs: Long, clock: (Long) -> String): String = when {
        !c.open -> "closed"
        c.untilConfigChange -> "open until the server settings change (${c.reason ?: "?"})"
        c.retryInMs != null -> "open until ${clock(nowMs + c.retryInMs)} (${c.reason ?: "?"})"
        else -> "open (${c.reason ?: "?"})"
    }

    /** "Active engine (last dictation)" row. */
    fun lastDictation(r: CommitResult?): String {
        if (r == null) return "No dictation yet in this process."
        val route = r.route
        val engine = when (route?.engine) {
            "remote" -> "Server${route.server?.let { " $it" } ?: ""}"
            "local" -> "Client (on-device)"
            null -> "Client (no router info)"
            else -> route.engine
        }
        val parts = mutableListOf(engine)
        route?.fallback?.let { parts += "fallback: $it" }
        route?.rttMs?.let { parts += "rtt $it ms" }
        route?.queueMs?.let { parts += "queue $it ms" }
        parts += "asr ${r.timings.asrMs} ms / total ${r.timings.totalMs} ms"
        parts += r.status.name + (r.emptyReason?.let { " ($it)" } ?: "") + (r.error?.let { ": $it" } ?: "")
        return parts.joinToString(" · ")
    }

    /** Bubble badge letter: S / L, with fallback flagged (§5 `show_engine_badge`). */
    fun badge(route: RouteInfo?): String? = when (route?.engine) {
        "remote" -> "S"
        "local" -> if (route.fallback != null) "L•" else "L"
        else -> null
    }
}
