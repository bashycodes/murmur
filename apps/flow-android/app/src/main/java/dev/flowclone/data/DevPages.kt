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
    /** D28: the cloud has its own page (ui/OpenRouterActivity), not a sub-page of this form. */
    enum class Page { HUB, ROUTING, SERVER, DEBUG, STATUS }

    private val SERVER_FIELDS = setOf(
        Field.HOST, Field.PORT, Field.PIN, Field.TOKEN, Field.CONNECT_TIMEOUT, Field.COMMIT_TIMEOUT, Field.HEALTH_TTL,
    )

    /** Form fields (with inline errors) shown on [p]. */
    fun fields(p: Page): Set<Field> = when (p) {
        Page.SERVER -> SERVER_FIELDS
        else -> emptySet()
    }

    fun pageOf(@Suppress("UNUSED_PARAMETER") f: Field): Page = Page.SERVER

    /** Errors this page cannot show inline (e.g. an out-of-range stored value on another page). */
    fun offPageErrors(r: DevSettingsForm.Result, p: Page): Map<Field, String> = r.errors.filterKeys { it !in fields(p) }

    private val SERVER_WARNINGS = setOf(DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN)
    /**
     * The form's warnings that belong on [p]. "Dictation disabled" is on the hub and Routing;
     * the cloud's (no consent, not in the order, fallback position) on Routing, where its order is.
     */
    fun warningsFor(p: Page, warnings: List<String>): List<String> = warnings.filter { w ->
        val server = w in SERVER_WARNINGS; val off = w == DevSettingsForm.BOTH_OFF
        when (p) {
            Page.HUB -> off
            Page.ROUTING -> !server
            Page.SERVER -> server
            else -> false
        }
    }

    /** [base] with the inputs [p] edits taken from [edits]. The cloud part is never page-owned (OpenRouter page). */
    fun merge(base: Input, edits: Input, p: Page): Input = when (p) {
        Page.ROUTING -> base.copy(
            allowServer = edits.allowServer, allowClient = edits.allowClient, routeOrder = edits.routeOrder,
            keepLocalWarm = edits.keepLocalWarm, showEngineBadge = edits.showEngineBadge,
        )
        Page.SERVER -> base.copy(
            host = edits.host, port = edits.port, tls = edits.tls, pin = edits.pin, token = edits.token,
            connectTimeoutMs = edits.connectTimeoutMs, commitTimeoutBaseMs = edits.commitTimeoutBaseMs, healthTtlS = edits.healthTtlS,
        )
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

    /** The store changed under the page (OpenRouter page, consent): keep this page's edits. */
    fun rebase(stored: DevSettings): DevDraft {
        val s = Input.of(stored)
        if (!dirty) return DevDraft(s, s, page)
        // An order this page did not change follows the store (the OpenRouter page puts the cloud first).
        val edits = if (draft.routeOrder == saved.routeOrder) draft.copy(routeOrder = s.routeOrder) else draft
        return DevDraft(s, DevPages.merge(s, edits, page), page)
    }

    companion object {
        fun of(s: DevSettings, page: DevPages.Page) = Input.of(s).let { DevDraft(it, it, page) }
    }
}

/** Hub row subtitles and the status card: the current value at a glance. Never the token or a key. */
object DevSummary {
    fun routeName(r: Route, s: DevSettings): String = when (r) {
        Route.SERVER -> "Server"
        Route.CLIENT -> "Phone"
        Route.CLOUD -> "Cloud (${OpenRouter.NAME})"
    }

    fun routing(s: DevSettings, caps: BuildCaps): String {
        val e = s.effective(caps)
        if (e.bothOff) return "Dictation disabled"
        val names = e.routeOrder.filter {
            when (it) {
                Route.SERVER -> e.allowServer
                Route.CLIENT -> e.allowClient
                Route.CLOUD -> e.cloudRouted
            }
        }.map { routeName(it, e) }
        // The stored order, made obvious: "Phone first → Server" / "Server only".
        return if (names.size == 1) "${names[0]} only" else names.mapIndexed { i, n -> if (i == 0) "$n first" else n }.joinToString(" → ")
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

    /** OpenRouter row: "Speech on · Cleanup off · budget $5.00" (+ "no key"). */
    fun cloud(s: DevSettings, caps: BuildCaps, hasKey: Boolean): String {
        if (!caps.cloud) return CloudText.NOT_IN_BUILD.replaceFirstChar { it.uppercase() }
        fun use(on: Boolean, consent: Boolean) = when { !on -> "off"; !consent -> "on, no consent"; else -> "on" }
        return listOfNotNull(
            "Speech ${use(s.cloudRouted, s.cloud.stt.consentOk)}",
            "Cleanup ${use(s.cloudCleanupRouted, s.cloud.cleanup.consentOk)}",
            if (!hasKey) "no key" else null,
            s.cloud.monthlyBudgetUsd?.let { "budget ${CloudText.usd(it)}" },
        ).joinToString(" · ")
    }

    fun debug(log: Boolean, audio: Boolean) = "Log ${if (log) "on" else "off"} · audio ${if (audio) "kept" else "off"}"

    /** Status card: engine of the last dictation + the phone-side latency (client_ms), else total. */
    fun lastShort(r: CommitResult?): String? {
        if (r == null) return null
        val route = r.route
        val engine = when (route?.engine) {
            "remote" -> "Server"
            "local", null -> "On-device"
            "cloud" -> "Cloud (${OpenRouter.NAME})"
            else -> route.engine
        } + (route?.fallback?.let { " (fallback: $it)" } ?: "")
        val ms = route?.clientMs?.let { "client $it ms" } ?: "total ${r.timings.totalMs} ms"
        return "$engine · $ms"
    }
}
