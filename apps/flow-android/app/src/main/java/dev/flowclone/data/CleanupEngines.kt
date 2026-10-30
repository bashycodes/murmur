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

    fun name(r: CleanupRoute): String = when (r) {
        CleanupRoute.LOCAL -> "This phone"
        CleanupRoute.SERVER -> "Your server"
        CleanupRoute.CLOUD -> OpenRouter.NAME
    }

    /** The switch's spoken label. */
    fun useLabel(r: CleanupRoute): String = when (r) {
        CleanupRoute.LOCAL -> "Clean up on this phone"
        CleanupRoute.SERVER -> "Clean up on your server"
        CleanupRoute.CLOUD -> "Clean up with ${OpenRouter.NAME}"
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

    /** OpenRouter row: "Sends text · asks first" until consent, then its model; "· no key". */
    fun cloudLine(d: DevSettings, hasKey: Boolean): String =
        "Sends text · " + (if (!d.cloud.cleanup.consentOk) "asks first" else d.cloud.cleanup.model.substringAfter('/')) +
            if (d.allowCloudCleanup && !hasKey) " · no key" else ""
}
