package dev.flowclone.data

import dev.flowclone.core.CommitResult
import dev.flowclone.data.DevSettings.Route
import dev.flowclone.data.DevSettingsForm.Field
import dev.flowclone.data.DevSettingsForm.Input

/**
 * Developer settings is a hub with focused sub-pages (ui/DeveloperActivity). Each page edits
 * its own slice of the one [DevSettingsForm.Input]; Save validates and writes the whole
 * [DevSettings] exactly as the old single page did, so prefs keys and validation are unchanged.
 */
object DevPages {
    enum class Page { HUB, ROUTING, SERVER, CLOUD, DEBUG, STATUS }

    private val SERVER_FIELDS = setOf(
        Field.HOST, Field.PORT, Field.PIN, Field.TOKEN, Field.CONNECT_TIMEOUT, Field.COMMIT_TIMEOUT, Field.HEALTH_TTL,
    )

    /** Form fields (with inline errors) shown on [p]. */
    fun fields(p: Page): Set<Field> = when (p) {
        Page.SERVER -> SERVER_FIELDS
        Page.CLOUD -> setOf(Field.CLOUD_BUDGET)
        else -> emptySet()
    }

    fun pageOf(f: Field): Page = if (f in SERVER_FIELDS) Page.SERVER else Page.CLOUD

    /** Errors this page cannot show inline (e.g. an out-of-range stored value on another page). */
    fun offPageErrors(r: DevSettingsForm.Result, p: Page): Map<Field, String> = r.errors.filterKeys { it !in fields(p) }

    private val SERVER_WARNINGS = setOf(DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN)
    private val CLOUD_WARNINGS = setOf(DevSettingsForm.CLOUD_NO_PROVIDER, DevSettingsForm.CLOUD_NO_CONSENT, DevSettingsForm.CLOUD_NOT_IN_ORDER)

    /**
     * The form's warnings that belong on [p]. "Dictation disabled" is on the hub and Routing;
     * route-order notes (the cloud's fallback position) on Routing and Cloud.
     */
    fun warningsFor(p: Page, warnings: List<String>): List<String> = warnings.filter { w ->
        val server = w in SERVER_WARNINGS; val cloud = w in CLOUD_WARNINGS; val off = w == DevSettingsForm.BOTH_OFF
        when (p) {
            Page.HUB -> off
            Page.ROUTING -> off || (!server && !cloud)
            Page.SERVER -> server
            Page.CLOUD -> cloud || (!server && !off)
            else -> false
        }
    }

    /** [base] with the inputs [p] edits taken from [edits]. Provider list / active id are never page-owned. */
    fun merge(base: Input, edits: Input, p: Page): Input = when (p) {
        Page.ROUTING -> base.copy(
            allowServer = edits.allowServer, allowClient = edits.allowClient, routeOrder = edits.routeOrder,
            keepLocalWarm = edits.keepLocalWarm, showEngineBadge = edits.showEngineBadge,
        )
        Page.SERVER -> base.copy(
            host = edits.host, port = edits.port, tls = edits.tls, pin = edits.pin, token = edits.token,
            connectTimeoutMs = edits.connectTimeoutMs, commitTimeoutBaseMs = edits.commitTimeoutBaseMs, healthTtlS = edits.healthTtlS,
        )
        // Enabling the cloud also puts it first in the order (§3.1 step 4), so the order is the cloud page's too.
        Page.CLOUD -> base.copy(allowCloud = edits.allowCloud, cloudBudget = edits.cloudBudget, routeOrder = edits.routeOrder)
        else -> base
    }

    /** Swap [r] with its visible neighbour; hidden routes keep their place in the stored order. */
    fun move(order: List<Route>, r: Route, dir: Int, shown: (Route) -> Boolean): List<Route> {
        val visible = order.filter(shown)
        val i = visible.indexOf(r); val j = i + dir
        if (i < 0 || j !in visible.indices) return order
        val other = visible[j]
        return order.map { when (it) { r -> other; other -> r; else -> it } }
    }
}

/** One sub-page's edit state: what is stored ([saved]) and what the page shows ([draft]). */
data class DevDraft(val saved: Input, val draft: Input, val page: DevPages.Page) {
    val dirty: Boolean get() = draft != saved

    fun edit(f: Input.() -> Input) = copy(draft = draft.f())

    fun result(caps: BuildCaps) = DevSettingsForm.validate(draft, caps)

    /** The top-bar Save: something changed and the whole form is valid. */
    fun canSave(caps: BuildCaps) = dirty && result(caps).canSave

    /** The store changed under the page (provider page, consent, Use): keep this page's edits. */
    fun rebase(stored: DevSettings): DevDraft {
        val s = Input.of(stored)
        return DevDraft(s, DevPages.merge(s, draft, page), page)
    }

    companion object {
        fun of(s: DevSettings, page: DevPages.Page) = Input.of(s).let { DevDraft(it, it, page) }
    }
}

/** Hub row subtitles and the status card: the current value at a glance. Never the token or a key. */
object DevSummary {
    fun routeName(r: Route, s: DevSettings): String = when (r) {
        Route.SERVER -> "Server"
        Route.CLIENT -> "On-device"
        Route.CLOUD -> s.cloud.activeProvider?.let { "Cloud (${it.displayName})" } ?: "Cloud"
    }

    fun routing(s: DevSettings, caps: BuildCaps): String {
        val e = s.effective(caps)
        if (e.bothOff) return "Dictation disabled"
        return e.routeOrder.filter {
            when (it) {
                Route.SERVER -> e.allowServer
                Route.CLIENT -> e.allowClient
                Route.CLOUD -> e.cloudRouted
            }
        }.joinToString(" → ") { routeName(it, e) }
    }

    private fun hostPort(s: DevSettings) = (if (s.host.contains(':')) "[${s.host}]" else s.host) + ":" + s.port

    fun server(s: DevSettings, caps: BuildCaps): String {
        if (!caps.remote) return "Not in this build"
        if (!s.serverConfigured) return "Not set"
        if (!s.allowServer) return "Off · ${hostPort(s)}"
        val e = s.effective(caps)
        return listOfNotNull(
            hostPort(s), if (e.tls) "TLS" else "ws://", if (e.tls && s.tlsPinSha256.isNotEmpty()) "pinned" else null,
            if (s.token.isEmpty()) "no token" else "token set",
        ).joinToString(" · ")
    }

    fun timeouts(s: DevSettings) = "connect ${s.connectTimeoutMs} ms · commit ${s.commitTimeoutBaseMs} ms · health ${s.healthTtlS} s"

    fun cloud(s: DevSettings, caps: BuildCaps): String {
        if (!caps.cloud) return CloudText.NOT_IN_BUILD.replaceFirstChar { it.uppercase() }
        val n = s.cloud.providers.size
        if (!s.allowCloud) return "Off · " + when (n) { 0 -> "no providers"; 1 -> "1 provider"; else -> "$n providers" }
        val p = s.cloud.activeProvider ?: return "On · no active provider"
        return listOfNotNull(
            "On", p.displayName, if (!p.consentOk) "no consent" else null,
            s.cloud.monthlyBudgetUsd?.let { "budget ${CloudText.usd(it)}" },
        ).joinToString(" · ")
    }

    fun debug(log: Boolean, audio: Boolean) = "Log ${if (log) "on" else "off"} · audio ${if (audio) "kept" else "off"}"

    /** Status card: engine of the last dictation + the phone-side latency (client_ms), else total. */
    fun lastShort(r: CommitResult?, providerLabel: (String) -> String? = { null }): String? {
        if (r == null) return null
        val route = r.route
        val engine = when (route?.engine) {
            "remote" -> "Server"
            "local", null -> "On-device"
            "cloud" -> "Cloud" + (route.provider?.let { id -> " (${providerLabel(id) ?: id})" } ?: "")
            else -> route.engine
        } + (route?.fallback?.let { " (fallback: $it)" } ?: "")
        val ms = route?.clientMs?.let { "client $it ms" } ?: "total ${r.timings.totalMs} ms"
        return "$engine · $ms"
    }
}
