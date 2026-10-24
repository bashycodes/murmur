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
     * Cleanup on (after its own consent): order cloud → phone AI when this build has an on-device
     * LLM, else cloud only. A user-chosen order that already has the cloud is kept.
     */
    fun enableCleanup(s: DevSettings, nowMs: Long, caps: BuildCaps): DevSettings = s.copy(
        allowCloudCleanup = true,
        cleanupOrder = if (CleanupRoute.CLOUD in s.cleanupOrder) s.cleanupOrder else defaultCleanupOrder(caps),
        cloud = if (s.cloud.cleanup.consentOk) s.cloud else s.cloud.copy(cleanup = s.cloud.cleanup.withConsent(nowMs)),
    )

    fun disableCleanup(s: DevSettings) = s.copy(allowCloudCleanup = false)

    fun defaultCleanupOrder(caps: BuildCaps): List<CleanupRoute> =
        if (caps.local) DevSettings.CLEANUP_CLOUD_FIRST else listOf(CleanupRoute.CLOUD)

    /** "Cloud first" = [cloud, local]; "Phone AI first" = [local, cloud] (cloud only when no phone model is loaded). */
    fun withCleanupOrder(s: DevSettings, cloudFirst: Boolean) =
        s.copy(cleanupOrder = if (cloudFirst) DevSettings.CLEANUP_CLOUD_FIRST else listOf(CleanupRoute.LOCAL, CleanupRoute.CLOUD))

    fun cleanupCloudFirst(s: DevSettings) = s.cleanupOrder.firstOrNull() != CleanupRoute.LOCAL

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

    /** The AI cleanup row: "OpenRouter (llama-3.3-70b-instruct), then on-device" / null when cloud cleanup is off. */
    fun cleanupEngineLine(s: DevSettings): String? {
        if (!(s.cloudCleanupRouted && s.cloud.cleanup.consentOk)) return null
        val cloud = "${OpenRouter.NAME} (${s.cloud.cleanup.model.substringAfter('/')})"
        return when (s.cleanupOrder.firstOrNull()) {
            CleanupRoute.LOCAL -> "On-device, then $cloud"
            else -> if (CleanupRoute.LOCAL in s.cleanupOrder) "$cloud, then on-device" else cloud
        }
    }
}
