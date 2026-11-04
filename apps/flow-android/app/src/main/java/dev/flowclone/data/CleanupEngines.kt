package dev.flowclone.data

import dev.flowclone.data.DevSettings.CleanupRoute

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
     * Phone row: [model] = the imported GGUF file name (null = none); [loaded] = the engine has it
     * loaded now (null = the engine is still loading).
     */
    fun phoneLine(llmEnabled: Boolean, model: String?, loaded: Boolean?): String {
        val m = SettingsSummary.shortModel(model) ?: return "No model imported"
        return when {
            !llmEnabled -> "$m · not loaded"
            loaded == null -> "$m · loading…"
            loaded -> "$m · loaded"
            else -> "$m · failed to load"
        }
    }

    /** Server row from `cleanup_status` ([serverLlm] null = not connected yet; [circuitOpen] = recently failing). */
    fun serverLine(d: DevSettings, caps: BuildCaps, serverLlm: Boolean?, circuitOpen: Boolean = false): String = when {
        !caps.remote -> "Not in this build"
        !d.serverConfigured -> "Not set up"
        circuitOpen -> "Not reachable"
        serverLlm == true -> "Has an AI model"
        serverLlm == false -> "No AI model on the server"
        else -> "Not connected yet"
    }

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
