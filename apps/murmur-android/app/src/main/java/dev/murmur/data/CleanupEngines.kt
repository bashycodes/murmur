package dev.murmur.data

import dev.murmur.core.HealthReport
import dev.murmur.data.DevSettings.CleanupRoute

/**
 * D36 Settings → AI cleanup's ranked engine list (ui/CleanupPage), the same widget as Speech to
 * text's: this build's engines in the stored [DevSettings.cleanupOrder], each with a switch, "Change
 * order" for ↑ / ↓. Each dictation is cleaned by the first switched-on engine that can (phone
 * model loaded; server reachable with an AI model; OpenRouter consented, keyed, in budget).
 *
 * The phone's switch is also the on-device AI itself ([Settings.llmEnabled]: off unloads the
 * model), so it shows on only when both are on.
 */
object CleanupEngines {
    /** This build's engines in the stored order. */
    fun engines(d: DevSettings, caps: BuildCaps): List<CleanupRoute> =
        DevSettings.fullCleanupOrder(d.cleanupOrder).filter { DevSettings.cleanupAvailable(it, caps) }

    fun on(d: DevSettings, s: Settings, r: CleanupRoute): Boolean = when (r) {
        CleanupRoute.LOCAL -> s.llmEnabled && d.cleanupOn(r)
        CleanupRoute.SERVER -> d.cleanupOn(r)
        CleanupRoute.CLOUD -> d.cloudCleanupRouted
    }

    /**
     * The engines a dictation's AI cleanup will try, in order, as the router plans it ([d] = the
     * effective settings; [phoneAi] = the on-device AI is on in a build that has it): switched on,
     * the server set up and not known to lack an AI model ([serverLlm] = `cleanup_status.server_llm`,
     * null = not known yet), the cloud consented and usable. Empty = no AI cleanup.
     */
    fun planned(d: DevSettings, phoneAi: Boolean, serverLlm: Boolean? = null): List<CleanupRoute> = d.cleanupOrder.filter {
        d.cleanupOn(it) && when (it) {
            CleanupRoute.LOCAL -> phoneAi
            CleanupRoute.SERVER -> d.serverConfigured && serverLlm != false
            CleanupRoute.CLOUD -> d.cloudCleanupRouted && d.cloud.cleanupConsentOk && !d.cloud.customNotSetUp
        }
    }

    /** A [planned] engine in a summary line: "this phone" / "your server" / its provider ("OpenRouter", "Custom server (10.0.0.5)"). */
    fun planName(r: CleanupRoute, d: DevSettings): String = when (r) {
        CleanupRoute.LOCAL -> "this phone"
        CleanupRoute.SERVER -> "your server"
        CleanupRoute.CLOUD -> d.cloud.cleanupProviderName
    }

    /**
     * The drawer's AI cleanup line: the [planned] engines, OpenRouter with its model ("This phone,
     * then your server, then OpenRouter (llama-3.3-70b-instruct)"); null when there are none.
     */
    fun engineLine(d: DevSettings, plan: List<CleanupRoute>): String? = plan.takeIf { it.isNotEmpty() }?.joinToString(", then ") {
        if (it == CleanupRoute.CLOUD && !d.cloud.customProvider) "${OpenRouter.NAME} (${d.cloud.cleanup.model.substringAfter('/')})"
        else planName(it, d)
    }?.replaceFirstChar { it.uppercase() }

    /** "Change order": [r] swapped with its listed neighbour; engines this build lacks keep their place. */
    fun reorder(d: DevSettings, caps: BuildCaps, r: CleanupRoute, dir: Int): List<CleanupRoute> {
        val order = DevSettings.fullCleanupOrder(d.cleanupOrder)
        val shown = order.filter { DevSettings.cleanupAvailable(it, caps) }
        val i = shown.indexOf(r); val j = i + dir
        if (i < 0 || j !in shown.indices) return order
        val other = shown[j]
        return order.map { when (it) { r -> other; other -> r; else -> it } }
    }

    /** D52: the cloud engine is named by its provider ("OpenRouter" / "Custom server (10.0.0.5)"). */
    fun name(r: CleanupRoute, d: DevSettings? = null): String = when (r) {
        CleanupRoute.LOCAL -> "This phone"
        CleanupRoute.SERVER -> "Your server"
        CleanupRoute.CLOUD -> cloudName(d)
    }

    /** The row keeps "Custom server" when it is not set up; [cloudLine] then says why it is off. */
    private fun cloudName(d: DevSettings?) =
        if (d?.cloud?.customNotSetUp == true) CustomServer.NAME else d?.cloud?.cleanupProviderName ?: OpenRouter.NAME

    /** The switch's spoken label. */
    fun useLabel(r: CleanupRoute, d: DevSettings? = null): String = when (r) {
        CleanupRoute.LOCAL -> "Clean up on this phone"
        CleanupRoute.SERVER -> "Clean up on your server"
        CleanupRoute.CLOUD -> "Clean up with ${cloudName(d)}"
    }

    /**
     * Phone row: [on] = the row's switch ([on] LOCAL); [model] = the imported GGUF file name (null =
     * none); [loaded] = the engine has it loaded now (null = the engine is still loading). Off says
     * so, and that the switch turns it on: a model alone does nothing while the row is off.
     */
    fun phoneLine(on: Boolean, model: String?, loaded: Boolean?): String {
        val m = SettingsSummary.shortModel(model) ?: return "No model imported"
        return when {
            !on -> "$m · off, tap to use it"
            loaded == null -> "$m · loading…"
            loaded -> "$m · loaded"
            else -> "$m · failed to load"
        }
    }

    /**
     * A cleanup model made active when [hadModel] was false (none before): the phone engine is switched
     * on, both its row ([DevSettings.cleanupOff]) and the on-device AI ([Settings.llmEnabled]). Null
     * when it is on already or a model was there before (a switch turned off with a model stays off).
     */
    fun phoneOnForFirstModel(hadModel: Boolean, s: Settings, d: DevSettings): Pair<Settings, Set<CleanupRoute>>? =
        if (hadModel || (s.llmEnabled && d.cleanupOn(CleanupRoute.LOCAL))) null
        else s.copy(llmEnabled = true) to d.withCleanup(CleanupRoute.LOCAL, true).cleanupOff

    /**
     * Server row. `cleanup_status` first ([serverLlm]: the cleanup connection's hello, null = none yet;
     * [circuitOpen] = recently failing), else the last health probe ([h]: Server → Test connection's or
     * the settings pages'; [checking] while one runs): not set up / not reachable / connected / ready.
     */
    fun serverLine(
        d: DevSettings, caps: BuildCaps, serverLlm: Boolean?, circuitOpen: Boolean = false,
        h: HealthReport? = null, checking: Boolean = false,
    ): String = when {
        !caps.remote -> "Not in this build"
        !d.serverConfigured -> "Not set up"
        circuitOpen -> NOT_REACHABLE
        serverLlm == true -> READY
        serverLlm == false -> NO_LLM
        h == null -> if (checking) "Checking…" else "Not checked yet"
        !h.ok -> NOT_REACHABLE
        h.llm == true -> READY
        h.llm == false -> NO_LLM
        else -> "Connected"
    }

    private const val READY = "Ready · has an AI model"
    private const val NO_LLM = "Connected · no AI model on the server"
    private const val NOT_REACHABLE = "Not reachable"

    /**
     * Cloud row: "Sends text · asks first" until consent, then its model; "· no key" ([hasKey]: the
     * OpenRouter key; a custom server's key is optional). D52: "· unencrypted (http)" for an http:// server.
     */
    fun cloudLine(d: DevSettings, hasKey: Boolean): String {
        val c = d.cloud
        // D52: a selected custom server that is not usable sends nothing; say so, never "OpenRouter".
        if (c.customNotSetUp) return CustomServer.NOT_SET_UP
        val model = if (c.customProvider) c.customCleanup.model else c.cleanup.model.substringAfter('/')
        return "Sends text · " + (if (!c.cleanupConsentOk) "asks first" else model) +
            (if (c.customProvider && !c.customCleanup.https) " · unencrypted (http)" else "") +
            if (d.allowCloudCleanup && !hasKey && !c.customProvider) " · no key" else ""
    }
}
