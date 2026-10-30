package dev.flowclone.data

import dev.flowclone.data.DevSettings.CleanupRoute
import dev.flowclone.data.DevSettings.Route

/**
 * The OpenRouter page's state changes (ui/OpenRouterActivity, ui/ModelPickerActivity), pure for
 * the JVM tests. Each use has its own opt-in and its own consent record: turning one on never
 * turns the other on, and turning a use off keeps its consent (turning it on again asks nothing).
 */
object OpenRouterPage {
    /** Speech-to-text needs the consent dialog first. */
    fun sttNeedsConsent(s: DevSettings) = !s.cloud.stt.consentOk
    fun cleanupNeedsConsent(s: DevSettings) = !s.cloud.cleanup.consentOk

    /** STT on (after its consent): the cloud goes first in the route order (§3.1 step 4). */
    fun enableStt(s: DevSettings, nowMs: Long): DevSettings = s.copy(
        allowCloud = true,
        routeOrder = if (Route.CLOUD in s.routeOrder) s.routeOrder else DevSettings.withCloudFirst(s.routeOrder),
        cloud = if (s.cloud.stt.consentOk) s.cloud else s.cloud.copy(stt = s.cloud.stt.withConsent(nowMs)),
    )

    /** STT off: the route stays in the stored order (hidden while off), the consent is kept. */
    fun disableStt(s: DevSettings) = s.copy(allowCloud = false)

    /**
     * Cleanup on (after its own consent): the cloud entry of the cleanup order switched on where
     * it stands (D36: the order itself is set on Settings → AI cleanup).
     */
    fun enableCleanup(s: DevSettings, nowMs: Long): DevSettings = s.copy(
        allowCloudCleanup = true,
        cleanupOrder = DevSettings.fullCleanupOrder(s.cleanupOrder),
        cloud = if (s.cloud.cleanup.consentOk) s.cloud else s.cloud.copy(cleanup = s.cloud.cleanup.withConsent(nowMs)),
    )

    fun disableCleanup(s: DevSettings) = s.copy(allowCloudCleanup = false)

    /** Picker choice for one use. [sttPath] (STT only): auto | transcriptions | chat_audio. */
    fun withModel(s: DevSettings, use: CloudCatalog.Use, id: String, sttPath: String = "auto"): DevSettings {
        val m = id.trim()
        require(OpenRouter.validModelId(m)) { "invalid model id" }
        return when (use) {
            CloudCatalog.Use.STT -> s.copy(cloud = s.cloud.copy(stt = s.cloud.stt.copy(model = m, sttPath = sttPath)))
            CloudCatalog.Use.CLEANUP -> s.copy(cloud = s.cloud.copy(cleanup = s.cloud.cleanup.copy(model = m)))
        }
    }

    fun model(s: DevSettings, use: CloudCatalog.Use) = when (use) {
        CloudCatalog.Use.STT -> s.cloud.stt.model
        CloudCatalog.Use.CLEANUP -> s.cloud.cleanup.model
    }

    fun withPrivacy(s: DevSettings, zdr: Boolean = s.cloud.privacy.zdr, denyCollection: Boolean = s.cloud.privacy.denyDataCollection) =
        s.copy(cloud = s.cloud.copy(privacy = CloudPrivacy(zdr, if (denyCollection) "deny" else "allow")))

    fun withCleanup(s: DevSettings, f: CleanupSettings.() -> CleanupSettings) = s.copy(cloud = s.cloud.copy(cleanup = s.cloud.cleanup.f()))
    fun withStt(s: DevSettings, f: SttSettings.() -> SttSettings) = s.copy(cloud = s.cloud.copy(stt = s.cloud.stt.f()))

    /** Whether the catalogue may be fetched in the background: only while a cloud use is on (spec §2.2). */
    fun mayRefreshInBackground(s: DevSettings) = s.allowCloud || s.allowCloudCleanup

    /** Consent line of one use. */
    fun consentLine(okAtMs: Long, ok: Boolean, what: String, date: (Long) -> String): String =
        if (ok) "Given on ${date(okAtMs)} for ${OpenRouter.HOST}" else "Not given yet: turning $what on asks first"

    /** Settings → DICTATION "Cloud (OpenRouter)" and the AI cleanup row: "Speech on · Cleanup on (llama-3.3-70b-instruct)". */
    fun settingsSummary(s: DevSettings, hasKey: Boolean): String {
        val speech = if (s.cloudRouted && s.cloud.stt.consentOk) "Speech on" else "Speech off"
        val cleanup = if (s.cloudCleanupRouted && s.cloud.cleanup.consentOk) "Cleanup on (${s.cloud.cleanup.model.substringAfter('/')})" else "Cleanup off"
        return "$speech · $cleanup" + if (!hasKey && (s.allowCloud || s.allowCloudCleanup)) " · no key" else ""
    }

    /**
     * The Home tab's AI cleanup line while cloud cleanup is on: the switched-on engines in order
     * ("On-device, then your server, then OpenRouter (llama-3.3-70b-instruct)"); null when it is off.
     */
    fun cleanupEngineLine(s: DevSettings): String? {
        if (!(s.cloudCleanupRouted && s.cloud.cleanup.consentOk)) return null
        val names = s.cleanupOrder.filter { s.cleanupOn(it) }.mapNotNull {
            when (it) {
                CleanupRoute.LOCAL -> "on-device"
                CleanupRoute.SERVER -> if (s.serverConfigured) "your server" else null
                CleanupRoute.CLOUD -> "${OpenRouter.NAME} (${s.cloud.cleanup.model.substringAfter('/')})"
            }
        }
        return names.joinToString(", then ").replaceFirstChar { it.uppercase() }
    }
}
