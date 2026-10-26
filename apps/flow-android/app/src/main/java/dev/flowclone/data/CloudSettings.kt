package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * D28 (docs/proposals/openrouter-only-cloud.md): OpenRouter is the only cloud service, for
 * speech-to-text and for the opt-in text cleanup. Its address is fixed in the native library;
 * nothing here names a host to connect to (the consent records name it, as the router does).
 */
object OpenRouter {
    /** Secure-storage id of the one API key (`SecretVault("flow_cloud_openrouter")`). */
    const val ID = "openrouter"
    /** The host consent is given for (`consent_host`). */
    const val HOST = "openrouter.ai"
    const val NAME = "OpenRouter"
    const val DEFAULT_STT_MODEL = "openai/whisper-large-v3-turbo"
    const val DEFAULT_CLEANUP_MODEL = "meta-llama/llama-3.3-70b-instruct"
    const val CLEANUP_TIMEOUT_MIN = 800L
    const val CLEANUP_TIMEOUT_MAX = 8_000L
    const val KEY_HINT = "sk-or-…"

    private val MODEL_ID = Regex("^[a-z0-9._~/-]+(:[a-z]+)*$")

    /** `flow_router::cloud::valid_model_id`: free text is allowed (e.g. `:nitro`), within this shape. */
    fun validModelId(id: String) = id.length in 1..128 && MODEL_ID.matches(id)

    /** Inline error for a typed model id, or null when it is valid. */
    fun modelIdError(raw: String): String? {
        val id = raw.trim()
        return when {
            id.isEmpty() -> "Enter a model id, e.g. $DEFAULT_STT_MODEL"
            id.length > 128 -> "A model id is at most 128 characters"
            id.any { it.isUpperCase() } -> "Model ids are lower case"
            !validModelId(id) -> "Use the id shown on openrouter.ai: author/model, optionally :variant"
            else -> null
        }
    }
}

/** `cloud.openrouter.privacy`: shared by both uses. Default: zero-retention providers only, no data collection. */
data class CloudPrivacy(val zdr: Boolean = true, val dataCollection: String = "deny") {
    val denyDataCollection: Boolean get() = dataCollection != "allow"

    fun toJson(): JSONObject = JSONObject().apply { put("zdr", zdr); put("data_collection", dataCollection) }

    companion object {
        fun fromJson(o: JSONObject?) = CloudPrivacy(
            zdr = o?.optBoolean("zdr", true) ?: true,
            dataCollection = o?.optString("data_collection", "deny")?.ifEmpty { "deny" } ?: "deny",
        )
    }
}

/** `cloud.openrouter.stt` (`flow_router::SttConfig`). */
data class SttSettings(
    val model: String = OpenRouter.DEFAULT_STT_MODEL,
    /** auto | transcriptions | chat_audio */
    val sttPath: String = "auto",
    /** "" = follow the dictation languages. */
    val language: String = "",
    val sendDictionary: Boolean = true,
    /** "" | wav | flac */
    val formatOverride: String = "",
    val timeoutBaseMs: Long = 6_000,
    /** Host the user agreed to send audio to (STT consent dialog) and when; 0 = none. */
    val consentHost: String = "",
    val consentAtMs: Long = 0,
) {
    val consentOk: Boolean get() = consentAtMs > 0 && consentHost.equals(OpenRouter.HOST, ignoreCase = true)

    fun withConsent(nowMs: Long) = copy(consentHost = OpenRouter.HOST, consentAtMs = nowMs)
    fun withoutConsent() = copy(consentHost = "", consentAtMs = 0)

    fun toJson(): JSONObject = JSONObject().apply {
        put("model", model); put("stt_path", sttPath); put("language", language); put("send_dictionary", sendDictionary)
        put("format_override", formatOverride); put("timeout_base_ms", timeoutBaseMs)
        put("consent_host", consentHost); put("consent_at_ms", consentAtMs)
    }

    companion object {
        fun fromJson(o: JSONObject?): SttSettings {
            if (o == null) return SttSettings()
            return SttSettings(
                model = o.optString("model", OpenRouter.DEFAULT_STT_MODEL).ifEmpty { OpenRouter.DEFAULT_STT_MODEL },
                sttPath = o.optString("stt_path", "auto").ifEmpty { "auto" },
                language = o.optString("language"),
                sendDictionary = o.optBoolean("send_dictionary", true),
                formatOverride = o.optString("format_override"),
                timeoutBaseMs = o.optLong("timeout_base_ms", 6_000),
                consentHost = o.optString("consent_host"),
                consentAtMs = o.optLong("consent_at_ms", 0),
            )
        }
    }
}

/** `cloud.openrouter.cleanup` (`flow_router::CleanupConfig`); its consent is separate from STT's. */
data class CleanupSettings(
    val model: String = OpenRouter.DEFAULT_CLEANUP_MODEL,
    /** latency | throughput | price | "" (OpenRouter's load balancing) */
    val sort: String = "latency",
    val timeoutMs: Long = 2_500,
    val connectTimeoutMs: Long = 1_500,
    val sendSurroundingText: Boolean = false,
    val sendScreenWords: Boolean = false,
    val sendAppName: Boolean = false,
    val consentHost: String = "",
    val consentAtMs: Long = 0,
) {
    val consentOk: Boolean get() = consentAtMs > 0 && consentHost.equals(OpenRouter.HOST, ignoreCase = true)

    fun withConsent(nowMs: Long) = copy(consentHost = OpenRouter.HOST, consentAtMs = nowMs)
    fun withoutConsent() = copy(consentHost = "", consentAtMs = 0)

    fun toJson(): JSONObject = JSONObject().apply {
        put("model", model); put("sort", sort); put("timeout_ms", timeoutMs); put("connect_timeout_ms", connectTimeoutMs)
        put("send_surrounding_text", sendSurroundingText); put("send_screen_words", sendScreenWords); put("send_app_name", sendAppName)
        put("consent_host", consentHost); put("consent_at_ms", consentAtMs)
    }

    companion object {
        fun fromJson(o: JSONObject?): CleanupSettings {
            if (o == null) return CleanupSettings()
            return CleanupSettings(
                model = o.optString("model", OpenRouter.DEFAULT_CLEANUP_MODEL).ifEmpty { OpenRouter.DEFAULT_CLEANUP_MODEL },
                sort = o.optString("sort", "latency"),
                timeoutMs = o.optLong("timeout_ms", 2_500),
                connectTimeoutMs = o.optLong("connect_timeout_ms", 1_500),
                sendSurroundingText = o.optBoolean("send_surrounding_text", false),
                sendScreenWords = o.optBoolean("send_screen_words", false),
                sendAppName = o.optBoolean("send_app_name", false),
                consentHost = o.optString("consent_host"),
                consentAtMs = o.optLong("consent_at_ms", 0),
            )
        }
    }
}

/**
 * `RouterConfig.cloud`, schema 2 (`crates/flow-router/fixtures/router_config_cloud.json`). API
 * keys are never part of it (one key, [OpenRouter.ID], in [CloudKeyStore]).
 */
data class CloudSettings(
    val privacy: CloudPrivacy = CloudPrivacy(),
    val stt: SttSettings = SttSettings(),
    val cleanup: CleanupSettings = CleanupSettings(),
    /** One budget for both uses; null = none. */
    val monthlyBudgetUsd: Double? = null,
) {
    val isDefault: Boolean get() = this == CloudSettings()

    /** The `OpenRouterConfig` JSON (`nativeProbeCloud`, `nativeSetCloudKeyFor`). */
    fun openRouterJson(): JSONObject = JSONObject().apply {
        put("privacy", privacy.toJson()); put("stt", stt.toJson()); put("cleanup", cleanup.toJson())
    }

    fun toJson(): JSONObject = JSONObject().apply {
        put("schema", SCHEMA)
        put("openrouter", openRouterJson())
        put("monthly_budget_usd", monthlyBudgetUsd ?: JSONObject.NULL)
    }

    companion object {
        const val SCHEMA = 2

        /**
         * Schema 2; a schema-1 object (D18 provider list) reads as the defaults: it is migrated
         * once by [CloudMigration] with the router's own rules, never interpreted here.
         */
        fun fromJson(o: JSONObject?): CloudSettings {
            if (o == null || isLegacy(o)) return CloudSettings()
            val or = o.optJSONObject("openrouter")
            return CloudSettings(
                privacy = CloudPrivacy.fromJson(or?.optJSONObject("privacy")),
                stt = SttSettings.fromJson(or?.optJSONObject("stt")),
                cleanup = CleanupSettings.fromJson(or?.optJSONObject("cleanup")),
                monthlyBudgetUsd = if (!o.has("monthly_budget_usd") || o.isNull("monthly_budget_usd")) null else o.optDouble("monthly_budget_usd"),
            )
        }

        /** Same test as `CloudConfig::is_legacy`: schema < 2 (missing = 1), or a D18 active id / provider list. */
        fun isLegacy(o: JSONObject): Boolean =
            o.optInt("schema", 1) < SCHEMA || o.optString("active").isNotEmpty() || (o.optJSONArray("providers")?.length() ?: 0) > 0
    }
}

/** flow_router `CloudUsage` (this month's counters; persisted by [DevStore.cloudUsageJson]). */
data class CloudUsage(val month: String = "", val providers: List<ProviderUsage> = emptyList()) {
    data class ProviderUsage(
        val providerId: String,
        val requests: Long = 0,
        val billedAudioMs: Long = 0,
        val costUsd: Double = 0.0,
        val costEstimated: Boolean = false,
        /** D28: "stt" (also when absent) | "cleanup". */
        val purpose: String = PURPOSE_STT,
    )

    val totalCostUsd: Double get() = providers.sumOf { it.costUsd }

    /** `CloudUsage::cost_for(purpose)`; counters of removed providers count as speech-to-text. */
    fun costFor(purpose: String): Double = providers.filter { it.purpose == purpose }.sumOf { it.costUsd }
    fun estimated(purpose: String): Boolean = providers.any { it.purpose == purpose && it.costEstimated }
    fun requests(purpose: String): Long = providers.filter { it.purpose == purpose }.sumOf { it.requests }
    fun billedAudioMs(): Long = providers.filter { it.purpose == PURPOSE_STT }.sumOf { it.billedAudioMs }

    fun toJson(): JSONObject = JSONObject().apply {
        put("month", month)
        put("providers", JSONArray().apply {
            for (p in providers) put(JSONObject().apply {
                put("provider_id", p.providerId); put("purpose", p.purpose); put("requests", p.requests)
                put("billed_audio_ms", p.billedAudioMs); put("cost_usd", p.costUsd); put("cost_estimated", p.costEstimated)
            })
        })
    }

    companion object {
        const val PURPOSE_STT = "stt"
        const val PURPOSE_CLEANUP = "cleanup"
        /** `flow_router::cloud::REMOVED_PROVIDERS_ID`: the month's counters of providers D28 removed. */
        const val REMOVED_ID = "removed"

        fun decode(json: String?): CloudUsage = runCatching {
            val o = JSONObject(json ?: return CloudUsage())
            val a = o.optJSONArray("providers") ?: JSONArray()
            CloudUsage(
                month = o.optString("month"),
                providers = (0 until a.length()).mapNotNull { i ->
                    a.optJSONObject(i)?.let {
                        ProviderUsage(
                            it.optString("provider_id"), it.optLong("requests"), it.optLong("billed_audio_ms"),
                            it.optDouble("cost_usd", 0.0).takeIf { c -> !c.isNaN() } ?: 0.0, it.optBoolean("cost_estimated"),
                            it.optString("purpose", PURPOSE_STT).ifEmpty { PURPOSE_STT },
                        )
                    }
                },
            )
        }.getOrDefault(CloudUsage())
    }
}

/** Cloud wording for the OpenRouter page, the Developer hub and the bubble; pure for the JVM tests. */
object CloudText {
    const val NOT_IN_BUILD = "not in this build"

    /** The page's first line: nothing is sent until a use is turned on. */
    const val INTRO = "OpenRouter (openrouter.ai) is the only cloud service this app can use. Nothing leaves this phone " +
        "until you turn on speech-to-text or text cleanup below, each with its own consent."

    const val PRIVACY_NOTE = "Zero-retention: OpenRouter only routes to providers that keep no copy of your audio or text. " +
        "Deny data collection: providers that may store or train on what you send are never used. OpenRouter itself keeps " +
        "request metadata (time, model, size, cost), not the content."

    const val ZDR_OFF_TITLE = "Allow providers that keep data?"
    const val ZDR_OFF_BODY = "Some providers may keep or train on your audio and text. Models without a zero-retention " +
        "provider become usable. \"Deny data collection\" stays on unless you turn it off too."
    const val COLLECTION_OFF_TITLE = "Allow providers that collect data?"
    const val COLLECTION_OFF_BODY = "Providers that may store your audio and text, or use them for training, can then serve your requests."

    const val STT_CONSENT_TITLE = "Send your voice to OpenRouter?"
    fun sttConsentBody(s: SttSettings): String =
        "When on, the audio of each dictation leaves this phone and is sent to OpenRouter (openrouter.ai) and to the " +
            "speech-to-text provider it routes to" + (if (s.sendDictionary) ", along with your dictionary words" else "") + ". " +
            "It is used before your server or the phone when it is first in the route order, or as a fallback otherwise. " +
            "Routed only to zero-retention providers by default.\n\nYou can turn this off at any time."
    const val STT_CONSENT_BUTTON = "Send audio to OpenRouter"

    /** Spec §2.5 wording + exactly what [c]'s PromptScope sends. */
    const val CLEANUP_CONSENT_TITLE = "Send dictated text to OpenRouter?"
    fun cleanupConsentBody(c: CleanupSettings): String =
        "After each dictation, the text (and your dictionary words) is sent to OpenRouter (openrouter.ai) and to the model " +
            "provider it routes to. Your voice is not sent unless Cloud speech-to-text is also on. Routed only to " +
            "zero-retention providers by default.\n\nWhat is sent: " + cleanupSends(c) + ".\n\nPassword fields are never " +
            "dictated into. Used for on-device and cloud dictations; your server uses its own AI. You can turn this off at any time."
    const val CLEANUP_CONSENT_BUTTON = "Send text to OpenRouter"

    /** PromptScope in words: the transcript, app category, style and dictionary always; each toggle adds its item. */
    fun cleanupSends(c: CleanupSettings): String {
        val items = mutableListOf("the dictated words", "the kind of app (e.g. chat, email)", "your writing style", "your dictionary words")
        if (c.sendSurroundingText) items += "the text around the cursor"
        if (c.sendScreenWords) items += "words on your screen"
        if (c.sendAppName) items += "the app's name"
        return items.dropLast(1).joinToString(", ") + " and " + items.last()
    }

    /** `••••abcd` for a stored key; never more than the last 4 characters. */
    fun mask(key: String?): String = when {
        key.isNullOrEmpty() -> "no key"
        key.length <= 4 -> "••••"
        else -> "••••" + key.takeLast(4)
    }

    /** Failure / `blocked` code → short hint (Test, Check key, fallback toast, cleanup status). */
    fun hint(reason: String?): String? = when (reason) {
        "auth_failed" -> "key rejected"
        "quota_exceeded" -> "out of OpenRouter credits"
        "connect_failed" -> "no internet"
        "budget_exceeded" -> "monthly budget reached"
        "cloud_not_configured" -> "add a key and accept the consent"
        "cloud_cleanup_not_configured" -> "turn on cleanup, accept its consent and add a key"
        "no_private_route" -> "no zero-retention provider serves this model: pick one marked ZDR"
        "busy" -> "rate limited, try again"
        "timeout" -> "too slow"
        "provider_error" -> "provider error"
        "circuit_open" -> "paused after earlier failures"
        else -> null
    }

    fun usd(v: Double): String = when {
        v == 0.0 -> "$0"
        v < 0.01 -> "$" + String.format(java.util.Locale.US, "%.4f", v)
        else -> "$" + String.format(java.util.Locale.US, "%.2f", v)
    }

    /** "cloud · OpenRouter · openai/whisper-large-v3-turbo · ≈$0.0012" (Active engine row). */
    fun engineRow(route: dev.flowclone.core.RouteInfo): String {
        val parts = mutableListOf("cloud", OpenRouter.NAME)
        route.model?.let { parts += it }
        route.costUsd?.let { parts += (if (route.costEstimated == true) "≈" else "") + usd(it) }
        return parts.joinToString(" · ")
    }

    /**
     * "AI cleanup: OpenRouter · llama-3.3-70b-instruct · $0.0007" (null when no LLM engine is reported).
     * D30: when the cloud answer was not used, the route actually used and why:
     * "AI cleanup: on-device · qwen · OpenRouter not used (llm_timeout)" / "AI cleanup: fixed rules · … ".
     */
    fun cleanupRow(route: dev.flowclone.core.RouteInfo?): String? {
        route ?: return null
        val fellBack = route.cleanupRoute != null && route.cleanupRoute != "cloud"
        if (fellBack) {
            val who = if (route.cleanupRoute == "local") "on-device" else "fixed rules"
            val model = route.llmModel?.takeIf { route.llmEngine == "local" }?.substringAfter('/')
            return listOfNotNull("AI cleanup: $who", model, "${OpenRouter.NAME} not used (${route.cloudFallback ?: "?"})",
                route.llmCostUsd?.takeIf { route.llmEngine == "cloud" }?.let(::usd)).joinToString(" · ")
        }
        val e = route.llmEngine ?: return null
        val who = when (e) { "cloud" -> OpenRouter.NAME; "local" -> "on-device"; "server" -> "server"; else -> e }
        return listOfNotNull("AI cleanup: $who", route.llmModel?.substringAfter('/'), route.llmCostUsd?.let(::usd)).joinToString(" · ")
    }

    /** USAGE card: "Speech $x · Cleanup $y" + the budget line. */
    fun usageLines(u: CloudUsage, s: CloudSettings): List<String> {
        fun part(purpose: String) = (if (u.estimated(purpose)) "≈ " else "") + usd(u.costFor(purpose))
        val stt = u.requests(CloudUsage.PURPOSE_STT)
        val cl = u.requests(CloudUsage.PURPOSE_CLEANUP)
        val lines = mutableListOf(
            "Speech ${part(CloudUsage.PURPOSE_STT)} · Cleanup ${part(CloudUsage.PURPOSE_CLEANUP)} · this month (${u.month.ifEmpty { "-" }})",
            "$stt speech request${if (stt == 1L) "" else "s"} (" + String.format(java.util.Locale.US, "%.1f min", u.billedAudioMs() / 60_000.0) +
                ") · $cl cleanup request${if (cl == 1L) "" else "s"}",
        )
        s.monthlyBudgetUsd?.let {
            lines += "Budget ${usd(it)}/month: ${usd(u.totalCostUsd)} used" + if (u.totalCostUsd >= it) " — reached, cloud paused until next month" else ""
        }
        return lines
    }

    /** §3.1 warnings for the route order. */
    fun orderWarning(order: List<DevSettings.Route>): String? {
        val i = order.indexOf(DevSettings.Route.CLOUD)
        if (i <= 0) return null
        return when (order[i - 1]) {
            DevSettings.Route.SERVER -> "When your server is unreachable, audio goes to OpenRouter."
            DevSettings.Route.CLIENT -> "When no on-device model is loaded, audio goes to OpenRouter."
            DevSettings.Route.CLOUD -> null
        }
    }

    /** Row name of the cloud route: "OpenRouter (whisper-large-v3-turbo)". */
    fun routeName(s: CloudSettings) = "${OpenRouter.NAME} (${s.stt.model.substringAfter('/')})"
}

/** The OpenRouter page's small text inputs (dialogs) → values, or the inline error. */
object CloudInput {
    data class Parsed<T>(val value: T?, val error: String?)

    /** Monthly budget in USD; blank = none. */
    fun budget(raw: String): Parsed<Double?> {
        val t = raw.trim().removePrefix("$").trim()
        if (t.isEmpty()) return Parsed(null, null)
        val v = t.toDoubleOrNull()?.takeIf { it >= 0.0 && it.isFinite() }
            ?: return Parsed(null, "Budget must be an amount in USD (e.g. 5 or 2.50), or empty for none")
        return Parsed(v, null)
    }

    fun budgetText(v: Double?): String = when {
        v == null -> ""
        v == Math.floor(v) -> v.toLong().toString()
        else -> v.toString()
    }

    /** Cloud cleanup timeout, 800-8000 ms (`CleanupConfig.timeout_ms`). */
    fun timeout(raw: String): Parsed<Long> {
        val v = raw.trim().toLongOrNull()
        if (v == null || v !in OpenRouter.CLEANUP_TIMEOUT_MIN..OpenRouter.CLEANUP_TIMEOUT_MAX) {
            return Parsed(null, "Timeout must be a whole number ${OpenRouter.CLEANUP_TIMEOUT_MIN}-${OpenRouter.CLEANUP_TIMEOUT_MAX} ms")
        }
        return Parsed(v, null)
    }

    /** STT language: "" = follow the dictation languages, else an ISO code (en, de, pt-br). */
    fun language(raw: String): Parsed<String> {
        val v = raw.trim().lowercase()
        if (v.isNotEmpty() && !Regex("^[a-z]{2,3}(-[a-z0-9]{2,8})?$").matches(v)) {
            return Parsed(null, "Language must be empty (follow your dictation languages) or a code like en, de, pt-br")
        }
        return Parsed(v, null)
    }

    /** `cleanup.sort` choices for the page. */
    val SORTS = listOf(
        "latency" to "Fastest provider (default)",
        "throughput" to "Highest throughput",
        "price" to "Cheapest provider",
        "" to "OpenRouter's load balancing",
    )

    fun sortName(sort: String) = SORTS.firstOrNull { it.first == sort }?.second ?: sort
}
