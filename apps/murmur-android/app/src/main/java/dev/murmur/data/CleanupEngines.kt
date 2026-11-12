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

    /**
     * [planned] for these settings: the phone's AI counts only where the build has it and it is on.
     * The one plan every cleanup summary (Settings row, drawer line) is formatted from.
     */
    fun plannedFor(s: Settings, d: DevSettings, caps: BuildCaps, serverLlm: Boolean? = null): List<CleanupRoute> =
        planned(d, s.llmEnabled && caps.local, serverLlm)

    /** A [planned] engine in a summary line: "this phone" / "your server" / "cloud" (D78: whichever provider). */
    fun planName(r: CleanupRoute): String = when (r) {
        CleanupRoute.LOCAL -> "this phone"
        CleanupRoute.SERVER -> "your server"
        CleanupRoute.CLOUD -> SettingsSummary.CLOUD.lowercase()
    }

    /** "Change order": [r] swapped with its listed neighbour; engines this build lacks keep their place. */
    fun reorder(d: DevSettings, caps: BuildCaps, r: CleanupRoute, dir: Int): List<CleanupRoute> {
        val order = DevSettings.fullCleanupOrder(d.cleanupOrder)
        val shown = order.filter { DevSettings.cleanupAvailable(it, caps) }
        val i = shown.indexOf(r); val j = i + dir
        if (i < 0 || j !in shown.indices) return order
        val other = shown[j]
        return order.map { when (it) { r -> other; other -> r; else -> it } }
    }

    /** D78: the cloud engine's row is "Cloud" whichever provider is picked; [cloudLine] names the provider. */
    fun name(r: CleanupRoute): String = when (r) {
        CleanupRoute.LOCAL -> "This phone"
        CleanupRoute.SERVER -> "Your server"
        CleanupRoute.CLOUD -> SettingsSummary.CLOUD
    }

    /** The switch's spoken label. */
    fun useLabel(r: CleanupRoute): String = when (r) {
        CleanupRoute.LOCAL -> "Clean up on this phone"
        CleanupRoute.SERVER -> "Clean up on your server"
        CleanupRoute.CLOUD -> "Clean up in the cloud"
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
     * Server row: not set up / not reachable / connected / ready. Unreachable first: an open circuit
     * ([circuitOpen] = recently failing) or a failed health probe ([h]: Server → Test connection's or the
     * settings pages', fresh on page open). The hello's "has a model" ([serverLlm], null = none yet) is
     * kept by the router with no expiry, so it must not hide a server that is down now. Then the hello,
     * else the probe ([checking] while one runs).
     */
    fun serverLine(
        d: DevSettings, caps: BuildCaps, serverLlm: Boolean?, circuitOpen: Boolean = false,
        h: HealthReport? = null, checking: Boolean = false,
    ): String = when {
        !caps.remote -> "Not in this build"
        !d.serverConfigured -> "Not set up"
        circuitOpen || h?.ok == false -> NOT_REACHABLE
        serverLlm == true -> READY
        serverLlm == false -> NO_LLM
        h == null -> if (checking) "Checking…" else "Not checked yet"
        h.llm == true -> READY
        h.llm == false -> NO_LLM
        else -> "Connected"
    }

    private const val READY = "Ready · has an AI model"
    private const val NO_LLM = "Connected · no AI model on the server"
    private const val NOT_REACHABLE = "Not reachable"

    /**
     * Cloud row: its provider, "Sends text · asks first" until consent, then its model; "· no key"
     * ([hasKey]: the OpenRouter key; LiteLLM's key is optional). D52: "· unencrypted (http)" for an
     * http:// proxy. "OpenRouter · Sends text · llama-3.3-70b-instruct" / "LiteLLM (10.0.0.5) · Sends text · gpt-4o-mini".
     */
    fun cloudLine(d: DevSettings, hasKey: Boolean): String {
        val c = d.cloud
        // D52: a selected LiteLLM that is not usable sends nothing; say so, never "OpenRouter".
        if (c.customNotSetUp) return CustomServer.NOT_SET_UP
        val model = if (c.customProvider) c.customCleanup.model else c.cleanup.model.substringAfter('/')
        return "${c.cleanupProviderName} · Sends text · " + (if (!c.cleanupConsentOk) "asks first" else model) +
            (if (c.customProvider && !c.customCleanup.https) " · unencrypted (http)" else "") +
            if (d.allowCloudCleanup && !hasKey && !c.customProvider) " · no key" else ""
    }
}
