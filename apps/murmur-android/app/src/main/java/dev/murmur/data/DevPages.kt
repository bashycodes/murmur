package dev.murmur.data

import dev.murmur.core.CommitResult
import dev.murmur.data.DevSettings.Route
import dev.murmur.data.DevSettingsForm.Field
import dev.murmur.data.DevSettingsForm.Input

/**
 * Developer settings is a hub with focused sub-pages (ui/DeveloperActivity). The Server page
 * edits its slice of the one [DevSettingsForm.Input]; Save validates and writes the whole
 * [DevSettings] exactly as the old single page did, so prefs keys and validation are unchanged.
 *
 * Which engines are on and their order moved to Settings → Speech to text (2026-10-30,
 * AndroidSpeech.dc.html): a ranked list whose switches and moves apply at once ([engines],
 * [engineOn], [withEngine], [reorder]). Developer keeps the engine extras on [Page.ENGINES].
 */
object DevPages {
    /** D28: the cloud has its own page (ui/OpenRouterActivity), not a sub-page of this form. */
    enum class Page { HUB, ENGINES, SERVER, DEBUG, STATUS }

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
     * The form's warnings that belong on [p]: the server's on the Server page, "no engine is on"
     * on the hub. The rest (engines, cloud consent and position) are on Speech to text ([speechWarnings]).
     */
    fun warningsFor(p: Page, warnings: List<String>): List<String> = warnings.filter { w ->
        when (p) {
            Page.HUB -> w == DevSettingsForm.BOTH_OFF
            Page.SERVER -> w in SERVER_WARNINGS
            else -> false
        }
    }

    const val NO_ENGINE = "No engine is on: turn one on to dictate."

    /** Settings → Speech to text banner: every warning that is not the server's ("no engine" reworded for the switches). */
    fun speechWarnings(warnings: List<String>): List<String> =
        warnings.filter { it !in SERVER_WARNINGS }.map { if (it == DevSettingsForm.BOTH_OFF) NO_ENGINE else it }

    /** [base] with the inputs [p] edits taken from [edits]. The cloud part is never page-owned (OpenRouter page). */
    fun merge(base: Input, edits: Input, p: Page): Input = when (p) {
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

    /** Engines this build has (server: `remote`, phone: `local`, OpenRouter: `cloud`). */
    fun available(r: Route, caps: BuildCaps) = when (r) {
        Route.SERVER -> caps.remote
        Route.CLIENT -> caps.local
        Route.CLOUD -> caps.cloud
    }

    /**
     * Speech to text's ranked list: the stored order, this build's engines only. The cloud is
     * in the stored order only once it was turned on or moved, so until then it is listed last.
     */
    fun engines(s: DevSettings, caps: BuildCaps): List<Route> {
        val listed = s.routeOrder.filter { available(it, caps) }.distinct()
        return listed + Route.entries.filter { available(it, caps) && it !in listed }
    }

    /** The engine's switch (the cloud counts as on once it is allowed and in the order). */
    fun engineOn(s: DevSettings, r: Route) = when (r) {
        Route.SERVER -> s.allowServer
        Route.CLIENT -> s.allowClient
        Route.CLOUD -> s.cloudRouted
    }

    /**
     * Server / phone switch. The cloud goes through its consent ([OpenRouterPage.enableStt]) after
     * [placeCloud], so it starts where the list shows it instead of jumping to the top.
     */
    fun withEngine(s: DevSettings, r: Route, on: Boolean): DevSettings = when (r) {
        Route.SERVER -> s.copy(allowServer = on)
        Route.CLIENT -> s.copy(allowClient = on)
        Route.CLOUD -> if (on) s.copy(allowCloud = true) else OpenRouterPage.disableStt(s)
    }

    /** The cloud put into the stored order at its listed place (before turning it on). */
    fun placeCloud(s: DevSettings, caps: BuildCaps): DevSettings =
        if (Route.CLOUD in s.routeOrder) s else s.copy(routeOrder = engines(s, caps) + s.routeOrder.filter { !available(it, caps) })

    /**
     * "Change order": [r] swapped with its listed neighbour. Engines this build lacks keep their
     * stored place at the end; a cloud that was never placed stays out of the order unless moved up.
     */
    fun reorder(s: DevSettings, caps: BuildCaps, r: Route, dir: Int): List<Route> {
        val moved = move(engines(s, caps), r, dir) { true }
        val out = moved + s.routeOrder.filter { !available(it, caps) }
        return if (Route.CLOUD !in s.routeOrder && moved.lastOrNull() == Route.CLOUD) out - Route.CLOUD else out
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
            "Cleanup ${use(s.cloudCleanupRouted, s.cloud.cleanupConsentOk)}",
            if (!hasKey) "no key" else null,
            s.cloud.monthlyBudgetUsd?.let { "budget ${CloudText.usd(it)}" },
        ).joinToString(" · ")
    }

    /** Engine options row: "Badge off · Parakeet greedy" ([parakeet] = decoding wire name, null without on-device). */
    fun engineOptions(badge: Boolean, parakeet: String?) =
        "Badge ${if (badge) "on" else "off"}" + (parakeet?.let { " · Parakeet $it" } ?: "")

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
