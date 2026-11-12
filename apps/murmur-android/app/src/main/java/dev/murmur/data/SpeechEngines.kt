package dev.murmur.data

import dev.murmur.data.DevSettings.Route

/**
 * Which speech-to-text engines a dictation can use, as [CleanupEngines] is for AI cleanup: the one
 * plan the Settings hub's Speech to text row and the drawer's line are both formatted from, so
 * they cannot disagree, and neither calls an engine ready that has nothing to run with.
 */
object SpeechEngines {
    /**
     * The phone can transcribe: the mock engine, or a default model the engine plan can load
     * ([defaultInstalled] = [EnginePlan.defaultInstalled]: whisper, or Parakeet when selected).
     */
    fun phoneReady(s: Settings, defaultInstalled: Boolean): Boolean = s.mockEngine || defaultInstalled

    /** The engines switched on in this build, in try order ([d] = the stored settings). */
    fun on(d: DevSettings, caps: BuildCaps): List<Route> {
        val e = d.effective(caps)
        return DevPages.engines(e, caps).filter { DevPages.engineOn(e, it) }
    }

    /**
     * The engines a dictation will try, in order: switched on and able to run ([phoneReady] for the
     * phone, a server address for your server, consent for the cloud). Empty = dictation cannot work yet.
     */
    fun planned(d: DevSettings, caps: BuildCaps, phoneReady: Boolean): List<Route> {
        val e = d.effective(caps)
        return on(d, caps).filter {
            when (it) {
                Route.SERVER -> e.serverConfigured
                Route.CLIENT -> phoneReady
                Route.CLOUD -> e.cloud.stt.consentOk
            }
        }
    }

    /** What an engine that is on but not [planned] still needs, for the "Not set up" summary. */
    fun setupHint(r: Route): String = when (r) {
        Route.SERVER -> "add your server"
        Route.CLIENT -> "import a speech model"
        Route.CLOUD -> "allow ${OpenRouter.NAME}"
    }
}
