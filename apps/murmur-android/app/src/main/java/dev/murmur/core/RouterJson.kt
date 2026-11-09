package dev.flowclone.core

import dev.flowclone.core.CoreJson.optDoubleOrNull
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

/** flow_router `CloudCapabilities` (`nativeCloudCapabilities()`, D18): does this library carry the cloud client. */
data class CloudCapabilities(val cloud: Boolean = false, val adapters: List<String> = emptyList())

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
    /** D18 Test provider (`engine:"cloud"`): provider id, model, what it heard, round trip, cost. */
    val provider: String? = null,
    val model: String? = null,
    val transcript: String? = null,
    val latencyMs: Long? = null,
    val costUsd: Double? = null,
)

data class CircuitStatus(
    val open: Boolean = false,
    val reason: String? = null,
    val retryInMs: Long? = null,
    /** auth_failed / proto_mismatch: stays open until the server config changes. */
    val untilConfigChange: Boolean = false,
)

/** flow_router `RouteStatus` (`nativeRouteStatus`). */
data class RouteStatus(
    val last: RouteInfo? = null,
    val circuit: CircuitStatus = CircuitStatus(),
    val health: HealthReport? = null,
    /** D18: the cloud route's own breaker; null in builds without the cloud. */
    val cloudCircuit: CircuitStatus? = null,
    /** D24: routing details of the last committed session. */
    val lastDiag: RouteDiag? = null,
    /** D28: the cloud cleanup's own breaker; null in builds without the cloud. */
    val cleanupCircuit: CircuitStatus? = null,
)

/** D28 `nativeCleanupStatus` (`flow_router::CleanupStatus`, fixture `cleanup_status.json`). */
data class CleanupStatus(
    val supported: Boolean = false,
    val enabled: Boolean = false,
    val order: List<String> = emptyList(),
    val consent: Boolean = false,
    /** Why a call now would be refused: cloud_cleanup_not_configured, circuit_open, auth_failed, quota_exceeded, no_private_route. */
    val blocked: String? = null,
    val circuit: CircuitStatus? = null,
    val model: String? = null,
    val timeoutMs: Long? = null,
    val last: CleanUsage? = null,
    val monthCostUsd: Double = 0.0,
    /** D52: the part of [monthCostUsd] the custom server reported; not in the OpenRouter budget. */
    val customMonthCostUsd: Double = 0.0,
    /** D36: the server has an AI model (`hello_ok.features`); null = not known yet / no server. */
    val serverLlm: Boolean? = null,
    /** D36: the server cleanup's breaker, when the server is in the order and set up. */
    val serverCircuit: CircuitStatus? = null,
    /** D52: "openrouter" | "custom" (the cloud cleanup route's service). */
    val provider: String = "openrouter",
    /** D52: the custom server's host; "" for OpenRouter. */
    val host: String = "",
)

/** D52 `nativeCustomCleanupModels` (`GET {base}/models`): the ids, or [error] + [message] (never the key or URL). */
data class CustomModels(
    val models: List<String> = emptyList(),
    val error: String? = null,
    val message: String? = null,
)

/** D28 `nativeCloudKeyInfo` (`GET /api/v1/key`; never the key itself), or [error]. */
data class CloudKeyInfo(
    val error: String? = null,
    val label: String? = null,
    val limit: Double? = null,
    val limitRemaining: Double? = null,
    val limitReset: String? = null,
    val usage: Double? = null,
    val usageMonthly: Double? = null,
    val isFreeTier: Boolean = false,
    val warnNoLimit: Boolean = false,
    val checkedAtMs: Long? = null,
)

/** Router JSON (D17 §1.2) ↔ Kotlin. Pinned by RouterJsonTest against crates/flow-router/fixtures. */
object RouterJson {
    private fun JSONObject.optIntOrNull(k: String): Int? = if (!has(k) || isNull(k)) null else optInt(k)
    private fun JSONObject.optBoolOrNull(k: String): Boolean? = if (!has(k) || isNull(k)) null else optBoolean(k)

    private fun JSONObject.optStr(k: String): String? = if (!has(k) || isNull(k)) null else optString(k)

    fun decodeSessionRoute(o: JSONObject?): SessionRoute? {
        val e = o?.optStr("engine") ?: return null
        return SessionRoute(e, o.optStr("provider"), o.optStr("model"), o.optStr("server"), o.optStr("fallback"))
    }

    /** `nativeStartSessionInfo`; a malformed answer still means "started" (route unknown). */
    fun decodeStartInfo(json: String): StartInfo = try {
        val o = JSONObject(json)
        StartInfo(o.optBoolean("queued"), decodeSessionRoute(o.optJSONObject("route")), o.optBoolean("loading"))
    } catch (e: org.json.JSONException) { StartInfo() }

    /** `nativePoll`; malformed = idle. */
    fun decodePoll(json: String?): CorePoll = try {
        val o = JSONObject(json ?: "{}")
        CorePoll(
            partial = o.optStr("partial"), endpointReached = o.optBoolean("endpoint_reached"),
            speechMs = o.optLong("speech_ms"), trailingSilenceMs = o.optLong("trailing_silence_ms"),
            route = decodeSessionRoute(o.optJSONObject("route")),
            loading = o.optBoolean("loading"),
        )
    } catch (e: org.json.JSONException) { CorePoll() }

    fun decodeRouteDiag(o: JSONObject?): RouteDiag? = o?.let {
        RouteDiag(
            connectMs = it.optLongOrNull("connect_ms"), localCold = it.optBoolean("local_cold"),
            localLoadMs = it.optLongOrNull("local_load_ms"), localLoadError = it.optStringOrNull("local_load_error"),
        )
    }

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

    fun decodeCloudCapabilities(json: String): CloudCapabilities = try {
        val o = JSONObject(json)
        val a = o.optJSONArray("adapters")
        CloudCapabilities(o.optBoolean("cloud"), if (a == null) emptyList() else (0 until a.length()).map { a.getString(it) })
    } catch (e: JSONException) {
        CloudCapabilities()
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
        provider = o.optStringOrNull("provider"),
        model = o.optStringOrNull("model"),
        transcript = o.optStringOrNull("transcript"),
        latencyMs = o.optLongOrNull("latency_ms"),
        costUsd = o.optDoubleOrNull("cost_usd"),
    )

    fun decodeRouteStatus(json: String): RouteStatus = try {
        val o = JSONObject(json)
        fun circuit(c: JSONObject) = CircuitStatus(
            open = c.optBoolean("open"), reason = c.optStringOrNull("reason"),
            retryInMs = c.optLongOrNull("retry_in_ms"),
            untilConfigChange = c.optBoolean("until_config_change"),
        )
        RouteStatus(
            last = o.optJSONObject("last")?.let(CoreJson::decodeRoute),
            circuit = circuit(o.optJSONObject("circuit") ?: JSONObject()),
            health = o.optJSONObject("health")?.let(::decodeHealth),
            cloudCircuit = o.optJSONObject("cloud_circuit")?.let(::circuit),
            lastDiag = decodeRouteDiag(o.optJSONObject("last_diag")),
            cleanupCircuit = o.optJSONObject("cleanup_circuit")?.let(::circuit),
        )
    } catch (e: JSONException) {
        RouteStatus()
    }

    private fun circuitOf(c: JSONObject) = CircuitStatus(
        open = c.optBoolean("open"), reason = c.optStringOrNull("reason"),
        retryInMs = c.optLongOrNull("retry_in_ms"), untilConfigChange = c.optBoolean("until_config_change"),
    )

    fun decodeCleanupStatus(json: String?): CleanupStatus = try {
        val o = JSONObject(json ?: "{}")
        val order = o.optJSONArray("order")
        CleanupStatus(
            supported = o.optBoolean("supported"), enabled = o.optBoolean("enabled"),
            order = if (order == null) emptyList() else (0 until order.length()).map { order.optString(it) },
            consent = o.optBoolean("consent"), blocked = o.optStringOrNull("blocked"),
            circuit = o.optJSONObject("circuit")?.let(::circuitOf),
            model = o.optStringOrNull("model"), timeoutMs = o.optLongOrNull("timeout_ms"),
            last = CoreJson.decodeCleanUsage(o.optJSONObject("last")),
            monthCostUsd = o.optDoubleOrNull("month_cost_usd") ?: 0.0,
            customMonthCostUsd = o.optDoubleOrNull("custom_month_cost_usd") ?: 0.0,
            serverLlm = o.optBoolOrNull("server_llm"),
            serverCircuit = o.optJSONObject("server_circuit")?.let(::circuitOf),
            provider = o.optString("provider", "openrouter").ifEmpty { "openrouter" },
            host = o.optString("host"),
        )
    } catch (e: JSONException) {
        CleanupStatus()
    }

    fun decodeCustomModels(json: String?): CustomModels = try {
        val o = JSONObject(json ?: "{}")
        val a = o.optJSONArray("models")
        val error = o.optStringOrNull("error") ?: if (a == null) "error" else null
        CustomModels(
            models = if (a == null) emptyList() else (0 until a.length()).map { a.optString(it) }.filter { it.isNotEmpty() },
            error = error, message = o.optStringOrNull("message"),
        )
    } catch (e: JSONException) {
        CustomModels(error = "error")
    }

    /** D52 `nativeCustomCleanupAddress`. */
    fun decodeCustomAddress(json: String?): dev.flowclone.data.CustomAddress = try {
        val o = JSONObject(json ?: "{}")
        val url = o.optStringOrNull("url")
        dev.flowclone.data.CustomAddress(
            url = url, host = if (url == null) "" else o.optString("host"), modelOk = o.optBoolean("model_ok"),
            error = o.optStringOrNull("error") ?: if (url == null) "error" else null, message = o.optStringOrNull("message"),
        )
    } catch (e: JSONException) {
        dev.flowclone.data.CustomAddress(error = "error")
    }

    fun decodeKeyInfo(json: String?): CloudKeyInfo = try {
        val o = JSONObject(json ?: "{}")
        CloudKeyInfo(
            error = o.optStringOrNull("error"), label = o.optStringOrNull("label"),
            limit = o.optDoubleOrNull("limit"), limitRemaining = o.optDoubleOrNull("limit_remaining"),
            limitReset = o.optStringOrNull("limit_reset"), usage = o.optDoubleOrNull("usage"),
            usageMonthly = o.optDoubleOrNull("usage_monthly"), isFreeTier = o.optBoolean("is_free_tier"),
            warnNoLimit = o.optBoolean("warn_no_limit"), checkedAtMs = o.optLongOrNull("checked_at_ms"),
        )
    } catch (e: JSONException) {
        CloudKeyInfo(error = "error")
    }
}

/** User-facing lines for the Developer page (§5), kept pure for the JVM tests. */
object DevText {
    fun probe(h: HealthReport): String {
        if (h.engine == "cloud") return cloudProbe(h)
        if (!h.ok && RouterErrors.plaintextRefused(h.message)) return "${h.error ?: "connect_failed"}: ${RouterErrors.NEEDS_TLS}"
        if (h.ok) {
            val rtt = h.rttMs?.let { "median ${it.median} ms (${it.min}-${it.max})" } ?: "no RTT"
            return listOfNotNull("OK", rtt, h.serverVersion, h.asrBackend).joinToString(" · ")
        }
        val hint = when (h.error) {
            "auth_failed" -> "token rejected"
            "connect_failed" -> "is flow-server running / LAN or VPN up?"
            "proto_mismatch" -> "client and server protocol versions differ (server proto ${h.proto ?: "?"})"
            "server_not_configured" -> "set the server host first"
            "busy" -> "server busy, try again"
            "timeout" -> "server too slow to answer"
            else -> null
        }
        return listOfNotNull("${h.error ?: "error"}${hint?.let { ": $it" } ?: ""}", h.message).joinToString(" — ")
    }

    /** D28 "Check key": label, credit left, this month's usage; a warning for a key without a limit. */
    fun keyInfo(k: CloudKeyInfo): String {
        k.error?.let { e -> return "$e" + (dev.flowclone.data.CloudText.hint(e)?.let { ": $it" } ?: "") }
        val u = dev.flowclone.data.CloudText::usd
        val credit = when {
            k.limitRemaining != null && k.limit != null -> "credit left ${u(k.limitRemaining)} of ${u(k.limit)}"
            k.limitRemaining != null -> "credit left ${u(k.limitRemaining)}"
            else -> "no credit limit"
        }
        return listOfNotNull(
            "Key OK", k.label, credit, k.usageMonthly?.let { "this month ${u(it)}" }, if (k.isFreeTier) "free tier" else null,
        ).joinToString(" · ") + if (k.warnNoLimit) "\nThis key has no credit limit; OpenRouter recommends setting one." else ""
    }

    /** D28 cleanup status line: "On · llama-3.3-70b-instruct · last 640 ms $0.0007", or why it is skipped. */
    fun cleanupStatus(c: CleanupStatus): String {
        if (!c.supported) return "Not in this build"
        if (!c.enabled) return "Off"
        val why = c.blocked?.let { b -> "skipped: " + (dev.flowclone.data.CloudText.hint(b) ?: b) }
        val last = c.last?.let { l ->
            listOfNotNull("last", l.latencyMs?.let { "$it ms" }, l.ttftMs?.let { "first token $it ms" }, l.costUsd?.let { (if (l.costEstimated) "≈" else "") + dev.flowclone.data.CloudText.usd(it) }).joinToString(" ")
        }
        // D52: month_cost_usd counts both providers; OpenRouter's part and the custom server's are shown apart.
        val usd = dev.flowclone.data.CloudText::usd
        val openRouter = (c.monthCostUsd - c.customMonthCostUsd).coerceAtLeast(0.0)
        val isCustom = c.provider == "custom"
        val orLine = if (!isCustom) "this month ${usd(openRouter)}" else openRouter.takeIf { it > 0 }?.let { "OpenRouter this month ${usd(it)}" }
        val custom = c.customMonthCostUsd.takeIf { it > 0 }?.let { "${usd(it)} on your server this month (not in budget)" }
        val who = if (isCustom) dev.flowclone.data.CustomServer.NAME + (if (c.host.isEmpty()) "" else " (${c.host})") else null
        return listOfNotNull("On", who, c.model, why, last, orLine, custom).joinToString(" · ")
    }

    /** D18 "Test provider" result line. */
    fun cloudProbe(h: HealthReport): String {
        if (h.ok) {
            return listOfNotNull(
                "OK", h.transcript?.let { "heard \"$it\"" }, h.latencyMs?.let { "$it ms" },
                h.costUsd?.let { dev.flowclone.data.CloudText.usd(it) }, h.model,
            ).joinToString(" · ")
        }
        if (RouterErrors.plaintextRefused(h.message)) return "${h.error ?: "connect_failed"}: ${RouterErrors.CLOUD_NEEDS_TLS}"
        val hint = dev.flowclone.data.CloudText.hint(h.error)
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

    fun circuit(c: CircuitStatus, nowMs: Long, untilWhat: String = "the server settings change", clock: (Long) -> String): String = when {
        !c.open -> "closed"
        c.untilConfigChange -> "open until $untilWhat (${c.reason ?: "?"})"
        c.retryInMs != null -> "open until ${clock(nowMs + c.retryInMs)} (${c.reason ?: "?"})"
        else -> "open (${c.reason ?: "?"})"
    }

    /** "Active engine (last dictation)" row. */
    /** [cloudName]: who the cloud cleanup route called (`CloudSettings.cleanupProviderName`, D52). */
    fun lastDictation(r: CommitResult?, cloudName: String = dev.flowclone.data.OpenRouter.NAME): String {
        if (r == null) return "No dictation yet in this process."
        val route = r.route
        val engine = when (route?.engine) {
            "cloud" -> dev.flowclone.data.CloudText.engineRow(route)
            "remote" -> "Server${route.server?.let { " $it" } ?: ""}"
            "local" -> "Client (on-device)"
            null -> "Client (no router info)"
            else -> route.engine
        }
        val parts = mutableListOf(engine)
        route?.fallback?.let { parts += "fallback: $it" }
        route?.rttMs?.let { parts += "rtt $it ms" }
        route?.queueMs?.let { parts += "queue $it ms" }
        route?.clientMs?.let { parts += "client $it ms" }
        parts += "asr ${r.timings.asrMs} ms / total ${r.timings.totalMs} ms"
        if (r.timings.segments > 0) parts += "${r.timings.segments} seg (+${r.timings.preAsrMs} ms early)"
        if (r.timings.waitMs > 0) parts += "wait ${r.timings.waitMs} ms"
        parts += r.status.name + (r.emptyReason?.let { " ($it)" } ?: "") + (r.error?.let { ": " + if (RouterErrors.plaintextRefused(it)) RouterErrors.NEEDS_TLS else it } ?: "")
        // D27: llm_skipped_clean reads as a normal result ("AI not needed"), not a failure.
        dev.flowclone.data.LlmOutcome.label(dev.flowclone.data.LlmOutcome.code(r))?.let { parts += it }
        // D28: who cleaned it (on-device / OpenRouter) and what it cost.
        dev.flowclone.data.CloudText.cleanupRow(route, cloudName)?.let { parts += it }
        return parts.joinToString(" · ")
    }

    /**
     * What the bubble's preview line shows: the engine letter when `show_engine_badge` is on;
     * C whenever the last dictation went to a cloud provider, even with the badge off (D18 C7).
     */
    fun bubbleBadge(route: RouteInfo?, showEngineBadge: Boolean): String? =
        if (showEngineBadge) badge(route) else if (route?.engine == "cloud") "C" else null

    /** Bubble badge letter: S / L / C (D18 cloud), with fallback flagged (§5 `show_engine_badge`). */
    fun badge(route: RouteInfo?): String? = when (route?.engine) {
        "cloud" -> "C"
        "remote" -> "S"
        "local" -> if (route.fallback != null) "L•" else "L"
        else -> null
    }
}
