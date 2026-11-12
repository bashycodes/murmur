package dev.murmur.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * D28 (docs/proposals/openrouter-only-cloud.md): OpenRouter is the only cloud service, for
 * speech-to-text and for the opt-in text cleanup. Its address is fixed in the native library;
 * nothing here names a host to connect to (the consent records name it, as the router does).
 */
object OpenRouter {
    /** Secure-storage id of the one API key (`SecretVault("murmur_cloud_openrouter")`). */
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

    /** `murmur_router::cloud::valid_model_id`: free text is allowed (e.g. `:nitro`), within this shape. */
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

/** `cloud.openrouter.stt` (`murmur_router::SttConfig`). */
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

/** `cloud.openrouter.cleanup` (`murmur_router::CleanupConfig`); its consent is separate from STT's. */
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
 * D52 "Load models" / "Test connection": the shell passes only a key typed into the field. The
 * stored key is chosen by the router, and only for the consented host
 * (`Router::with_custom_cleanup_key`).
 */
object CustomModelsKey {
    fun typedOnly(typed: String?): String? = typed?.trim()?.takeIf { it.isNotEmpty() }
}

/** D52 `cloud.cleanup_provider`: which service the cloud cleanup route calls. STT is always OpenRouter. */
enum class CleanupProvider(val wire: String) {
    OPENROUTER("openrouter"),
    CUSTOM("custom");

    companion object {
        fun parse(w: String?) = entries.firstOrNull { it.wire == w } ?: OPENROUTER
    }
}

/**
 * D52 `nativeCustomCleanupAddress`: the router's own reading of a custom server address
 * (`murmur_router::cloud::custom_server_address` + `valid_custom_model_id`). [url] is the normalised
 * API root and [host] the canonical host consent binds to; null [url] = unusable ([message] says why).
 */
data class CustomAddress(
    val url: String? = null,
    val host: String = "",
    val modelOk: Boolean = false,
    val error: String? = null,
    val message: String? = null,
) {
    val https: Boolean get() = url?.startsWith("https://") ?: true
}

/** Where [CustomServer] gets the router's reading of an address (the native library; a fake in JVM tests). */
fun interface CustomAddressCheck {
    fun check(baseUrl: String, model: String): CustomAddress
}

/**
 * D52: the user's own OpenAI-compatible server for cleanup (LiteLLM proxy, vLLM, Ollama, …).
 * URL normalisation, the consent host and model-id rules come from the router through [check]
 * (never a Kotlin URL parser), so the page records consent for exactly the host the router checks.
 */
object CustomServer {
    /** Secure-storage id of its optional key (`SecretVault("murmur_cloud_custom")`, `set_cloud_key("custom")`). */
    const val ID = "custom"
    const val NAME = "Custom server"
    const val TIMEOUT_MIN = 800L
    const val TIMEOUT_MAX = 30_000L
    const val DEFAULT_TIMEOUT_MS = 4_000L
    /** Custom server selected but its address or model is unusable: the router gets no cleanup consent. */
    const val NOT_SET_UP = "Custom server not set up — cloud cleanup off"
    const val URL_HINT = "/v1 is added when no path is given; include the full path if your server uses a prefix."

    /** The router's reader; replaced in JVM tests. A library without D52 reads every address as unusable. */
    @Volatile var check: CustomAddressCheck = CustomAddressCheck { u, m -> dev.murmur.core.NativeMurmurCore.customCleanupAddress(u, m) }

    fun address(baseUrl: String, model: String = ""): CustomAddress = check.check(baseUrl, model)

    fun normalizeBaseUrl(raw: String): String? = address(raw).url

    /** The canonical host of [baseUrl] (the router's `CustomCleanupConfig::host`), or "" when unusable. */
    fun host(baseUrl: String): String = address(baseUrl).host

    /** `valid_custom_model_id`: free text, 1-256 bytes, no whitespace or control characters. */
    fun validModelId(id: String): Boolean = address("", id).modelOk
}

/**
 * `cloud.custom_cleanup` (`murmur_router::cloud::CustomCleanupConfig`). The consent is bound to the
 * host of [baseUrl]: pointing it at another host voids it.
 */
data class CustomCleanupSettings(
    /** Stored normalised ([CustomServer.normalizeBaseUrl]); "" = not set up. */
    val baseUrl: String = "",
    val model: String = "",
    val timeoutMs: Long = CustomServer.DEFAULT_TIMEOUT_MS,
    val connectTimeoutMs: Long = 1_500,
    val consentHost: String = "",
    val consentAtMs: Long = 0,
) {
    /**
     * The router's reading of [baseUrl] and [model], looked up once per settings value (one native
     * call per page refresh): [host], [https], [consentOk] and [ready] all derive from it.
     */
    private val address: CustomAddress by lazy { CustomServer.address(baseUrl, model.trim()) }

    val host: String get() = address.host
    val https: Boolean get() = address.https
    val consentOk: Boolean get() = consentAtMs > 0 && consentHost.isNotEmpty() && host.isNotEmpty() && consentHost.equals(host, ignoreCase = true)

    /** The router accepts it (`CustomCleanupConfig::validate`): a usable URL and model id. */
    val ready: Boolean
        get() = address.let { it.url != null && it.modelOk } && timeoutMs in CustomServer.TIMEOUT_MIN..CustomServer.TIMEOUT_MAX

    fun withConsent(nowMs: Long) = copy(consentHost = host, consentAtMs = nowMs)

    fun toJson(): JSONObject = JSONObject().apply {
        put("base_url", baseUrl); put("model", model); put("timeout_ms", timeoutMs); put("connect_timeout_ms", connectTimeoutMs)
        put("consent_host", consentHost); put("consent_at_ms", consentAtMs)
    }

    companion object {
        fun fromJson(o: JSONObject?): CustomCleanupSettings {
            if (o == null) return CustomCleanupSettings()
            return CustomCleanupSettings(
                baseUrl = o.optString("base_url"),
                model = o.optString("model"),
                timeoutMs = o.optLong("timeout_ms", CustomServer.DEFAULT_TIMEOUT_MS),
                connectTimeoutMs = o.optLong("connect_timeout_ms", 1_500),
                consentHost = o.optString("consent_host"),
                consentAtMs = o.optLong("consent_at_ms", 0),
            )
        }
    }
}

/**
 * `RouterConfig.cloud`, schema 2 (`crates/murmur-router/fixtures/router_config_cloud.json`). API
 * keys are never part of it (one key, [OpenRouter.ID], in [CloudKeyStore]).
 */
data class CloudSettings(
    val privacy: CloudPrivacy = CloudPrivacy(),
    val stt: SttSettings = SttSettings(),
    val cleanup: CleanupSettings = CleanupSettings(),
    /** One budget for both uses; null = none. */
    val monthlyBudgetUsd: Double? = null,
    /** D52: OpenRouter (default) or [customCleanup] for the cleanup route. */
    val cleanupProvider: CleanupProvider = CleanupProvider.OPENROUTER,
    val customCleanup: CustomCleanupSettings = CustomCleanupSettings(),
) {
    val isDefault: Boolean get() = this == CloudSettings()
    val customProvider: Boolean get() = cleanupProvider == CleanupProvider.CUSTOM

    /** `CloudConfig::cleanup_consent_ok`: the selected provider's own consent. */
    val cleanupConsentOk: Boolean get() = if (customProvider) customCleanup.consentOk else cleanup.consentOk
    val cleanupModel: String get() = if (customProvider) customCleanup.model else cleanup.model
    val cleanupTimeoutMs: Long get() = if (customProvider) customCleanup.timeoutMs else cleanup.timeoutMs
    /** The cleanup key's [CloudKeyStore] id: the custom key is optional, OpenRouter's is required. */
    val cleanupKeyId: String get() = if (customProvider) CustomServer.ID else OpenRouter.ID

    /** Custom server selected but not usable: shown as [CustomServer.NOT_SET_UP], never as OpenRouter. */
    val customNotSetUp: Boolean get() = customProvider && !customCleanup.ready

    /** "OpenRouter" / "Custom server (10.0.0.5)" / [CustomServer.NOT_SET_UP]: the cloud cleanup engine's name. */
    val cleanupProviderName: String
        get() = when {
            !customProvider -> OpenRouter.NAME
            customNotSetUp -> CustomServer.NOT_SET_UP
            else -> customCleanup.host.let { if (it.isEmpty()) CustomServer.NAME else "${CustomServer.NAME} ($it)" }
        }

    /** The `OpenRouterConfig` JSON (`nativeProbeCloud`, `nativeSetCloudKeyFor`). */
    fun openRouterJson(): JSONObject = JSONObject().apply {
        put("privacy", privacy.toJson()); put("stt", stt.toJson()); put("cleanup", cleanup.toJson())
    }

    fun toJson(): JSONObject = JSONObject().apply {
        put("schema", SCHEMA)
        put("openrouter", openRouterJson())
        put("monthly_budget_usd", monthlyBudgetUsd ?: JSONObject.NULL)
        // Omitted while default, like the router's `skip_serializing_if`: older readers see the same JSON.
        if (customProvider) put("cleanup_provider", cleanupProvider.wire)
        if (customCleanup != CustomCleanupSettings()) put("custom_cleanup", customCleanup.toJson())
    }

    /**
     * [toJson] for the router: a custom server that is not [CustomCleanupSettings.ready] would make
     * the router refuse the whole config, so it is sent as OpenRouter with no cleanup consent (no
     * cleanup leaves the phone until the server is set up). The pages then say
     * [CustomServer.NOT_SET_UP] ([customNotSetUp]), never "OpenRouter".
     */
    fun routerJson(): JSONObject =
        if (customProvider && !customCleanup.ready) copy(cleanupProvider = CleanupProvider.OPENROUTER, cleanup = cleanup.withoutConsent()).toJson()
        else toJson()

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
                cleanupProvider = CleanupProvider.parse(o.optString("cleanup_provider")),
                customCleanup = CustomCleanupSettings.fromJson(o.optJSONObject("custom_cleanup")),
            )
        }

        /** Same test as `CloudConfig::is_legacy`: schema < 2 (missing = 1), or a D18 active id / provider list. */
        fun isLegacy(o: JSONObject): Boolean =
            o.optInt("schema", 1) < SCHEMA || o.optString("active").isNotEmpty() || (o.optJSONArray("providers")?.length() ?: 0) > 0
    }
}

/** murmur_router `CloudUsage` (this month's counters; persisted by [DevStore.cloudUsageJson]). */
data class CloudUsage(val month: String = "", val providers: List<ProviderUsage> = emptyList()) {
    data class ProviderUsage(
        val providerId: String,
        val requests: Long = 0,
        val billedAudioMs: Long = 0,
        val costUsd: Double = 0.0,
        val costEstimated: Boolean = false,
        /** D28: "stt" (also when absent) | "cleanup". */
        val purpose: String = PURPOSE_STT,
    ) {
        /** D52: the custom cleanup server's counter (never OpenRouter's, never in the budget). */
        val isCustom: Boolean get() = providerId == CustomServer.ID
    }

    /** `CloudUsage::total_cost`: every provider, the custom server included (never shown as OpenRouter's). */
    val totalCostUsd: Double get() = providers.sumOf { it.costUsd }
    /** D52 `CloudUsage::budget_cost`: what the monthly budget counts (OpenRouter only, never the custom server). */
    val budgetCostUsd: Double get() = providers.filterNot { it.isCustom }.sumOf { it.costUsd }
    /** D52 `CloudUsage::custom_cost`: what the custom server reported (its cost header), shown apart. */
    val customCostUsd: Double get() = providers.filter { it.isCustom }.sumOf { it.costUsd }

    /**
     * D52: one use's OpenRouter part (what the budget counts), the custom server left out; counters
     * of removed providers count as speech-to-text.
     */
    fun openRouterCostFor(purpose: String): Double = openRouter(purpose).sumOf { it.costUsd }
    fun openRouterEstimated(purpose: String): Boolean = openRouter(purpose).any { it.costEstimated }
    fun openRouterRequests(purpose: String): Long = openRouter(purpose).sumOf { it.requests }
    /** D52: cleanup requests sent to the custom server this month (counted even when it reports no cost). */
    val customRequests: Long get() = providers.filter { it.isCustom }.sumOf { it.requests }
    private fun openRouter(purpose: String) = providers.filter { it.purpose == purpose && !it.isCustom }
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
        /** `murmur_router::cloud::REMOVED_PROVIDERS_ID`: the month's counters of providers D28 removed. */
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

    /** D52: the consent of a custom server names the typed host, not OpenRouter. */
    fun customConsentTitle(host: String) = "Send dictated text to $host?"
    fun customConsentBody(cc: CustomCleanupSettings, c: CleanupSettings): String =
        "After each dictation, the text (and your dictionary words) is sent to your server at ${cc.host}. Your voice is " +
            "not sent: cloud speech-to-text always uses OpenRouter.\n\nWhat is sent: " + cleanupSends(c) + "." +
            (if (!cc.https) "\n\n$HTTP_WARNING" else "") +
            "\n\nPassword fields are never dictated into. You can turn this off at any time."
    const val CUSTOM_CONSENT_BUTTON = "Send text to this server"

    /** Shown under an http:// base URL and in its consent. */
    const val HTTP_WARNING = "This address uses http://: the key and the dictated text are sent unencrypted."
    const val CUSTOM_STT_NOTE = "Only AI cleanup uses this server. Cloud speech-to-text always uses OpenRouter."

    /** The consent dialog of the selected provider: (title, body, button). */
    fun cleanupConsent(s: CloudSettings): Triple<String, String, String> =
        if (s.customProvider) Triple(customConsentTitle(s.customCleanup.host), customConsentBody(s.customCleanup, s.cleanup), CUSTOM_CONSENT_BUTTON)
        else Triple(CLEANUP_CONSENT_TITLE, cleanupConsentBody(s.cleanup), CLEANUP_CONSENT_BUTTON)

    /** "Load models" / "Test connection" result: the count, or the error's hint and the server's message. */
    fun customModelsLine(m: dev.murmur.core.CustomModels): String = when {
        m.error == null -> if (m.models.isEmpty()) "Connected: the server lists no models" else
            "Connected: ${m.models.size} model${if (m.models.size == 1) "" else "s"}"
        else -> "Failed: " + (hint(m.error)?.let { if (m.error == "connect_failed") "server not reachable" else it } ?: m.error) +
            (m.message?.takeIf { it.isNotBlank() }?.let { " ($it)" } ?: "")
    }

    /** PromptScope in words: the transcript, app category, style and dictionary always; each toggle adds its item. */
    fun cleanupSends(c: CleanupSettings): String {
        val items = mutableListOf("the dictated words", "the kind of app (e.g. chat, email)", "your writing style", "your dictionary words")
        if (c.sendSurroundingText) items += "the text around the cursor"
        if (c.sendScreenWords) items += "words on your screen"
        if (c.sendAppName) items += "the app's name"
        return Prose.list(items)
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
    fun engineRow(route: dev.murmur.core.RouteInfo): String {
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
    fun cleanupRow(route: dev.murmur.core.RouteInfo?, cloudName: String = OpenRouter.NAME): String? {
        route ?: return null
        val fellBack = route.cleanupRoute != null && route.cleanupRoute != "cloud"
        if (fellBack) {
            val who = if (route.cleanupRoute == "local") "on-device" else "fixed rules"
            val model = route.llmModel?.takeIf { route.llmEngine == "local" }?.substringAfter('/')
            return listOfNotNull("AI cleanup: $who", model, "$cloudName not used (${route.cloudFallback ?: "?"})",
                route.llmCostUsd?.takeIf { route.llmEngine == "cloud" }?.let(::usd)).joinToString(" · ")
        }
        val e = route.llmEngine ?: return null
        val who = when (e) { "cloud" -> cloudName; "local" -> "on-device"; "server" -> "server"; else -> e }
        return listOfNotNull("AI cleanup: $who", route.llmModel?.substringAfter('/'), route.llmCostUsd?.let(::usd)).joinToString(" · ")
    }

    /**
     * USAGE card header: OpenRouter this month (what the budget counts), its speech and cleanup
     * parts. D52: the custom server's usage is never in these; [usageLines] gives it its own line.
     */
    fun usageStats(u: CloudUsage, s: CloudSettings): List<Pair<String, String>> = listOf(
        usd(u.budgetCostUsd) to "this month" + (s.monthlyBudgetUsd?.let { " of ${usd(it)}" } ?: ""),
        openRouterCost(u, CloudUsage.PURPOSE_STT) to "speech",
        openRouterCost(u, CloudUsage.PURPOSE_CLEANUP) to "cleanup",
    )

    /** "≈$0.25": one use's OpenRouter cost, marked when estimated. */
    private fun openRouterCost(u: CloudUsage, purpose: String, approx: String = "≈") =
        (if (u.openRouterEstimated(purpose)) approx else "") + usd(u.openRouterCostFor(purpose))

    /** "1 cleanup request" / "2 cleanup requests". */
    private fun count(n: Long, what: String) = "$n $what" + if (n == 1L) "" else "s"

    /** USAGE card: OpenRouter's "Speech $x · Cleanup $y", its requests, the budget line, then the custom server's line. */
    fun usageLines(u: CloudUsage, s: CloudSettings): List<String> {
        fun part(purpose: String) = openRouterCost(u, purpose, approx = "≈ ")
        val lines = mutableListOf(
            "Speech ${part(CloudUsage.PURPOSE_STT)} · Cleanup ${part(CloudUsage.PURPOSE_CLEANUP)} · this month (${u.month.ifEmpty { "-" }})",
            count(u.openRouterRequests(CloudUsage.PURPOSE_STT), "speech request") + " (" +
                String.format(java.util.Locale.US, "%.1f min", u.billedAudioMs() / 60_000.0) + ") · " +
                count(u.openRouterRequests(CloudUsage.PURPOSE_CLEANUP), "cleanup request"),
        )
        s.monthlyBudgetUsd?.let {
            lines += "Budget ${usd(it)}/month: ${usd(u.budgetCostUsd)} used" + if (u.budgetCostUsd >= it) " — reached, OpenRouter paused until next month" else ""
        }
        // D52: the owner's own server is never in the OpenRouter budget; its requests count even without a cost.
        if (u.customRequests > 0 || u.customCostUsd > 0) {
            val cost = u.customCostUsd.takeIf { it > 0 }?.let { "reported ${usd(it)}" } ?: "no cost reported"
            lines += "${CustomServer.NAME}: ${count(u.customRequests, "cleanup request")} · $cost (not in the budget)"
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

    /** D52 base URL as typed → the router's normalised URL ([CustomServer.address]). */
    fun baseUrl(raw: String): Parsed<String> {
        if (raw.isBlank()) return Parsed(null, "Enter http(s)://host[:port][/path], e.g. http://192.168.1.20:4000")
        val a = CustomServer.address(raw)
        return if (a.url != null) Parsed(a.url, null)
        else Parsed(null, (a.message ?: "Not a valid http(s) URL").replaceFirstChar { it.uppercase() })
    }

    /** D52 model id: free text as the server lists it. */
    fun customModel(raw: String): Parsed<String> {
        val m = raw.trim()
        return when {
            m.isEmpty() -> Parsed(null, "Enter a model id the server lists, e.g. gpt-4o-mini")
            !CustomServer.validModelId(m) -> Parsed(null, "A model id is at most 256 characters, without spaces")
            else -> Parsed(m, null)
        }
    }

    /** D52 custom server timeout, 800-30000 ms. */
    fun customTimeout(raw: String): Parsed<Long> {
        val v = raw.trim().toLongOrNull()
        if (v == null || v !in CustomServer.TIMEOUT_MIN..CustomServer.TIMEOUT_MAX) {
            return Parsed(null, "Timeout must be a whole number ${CustomServer.TIMEOUT_MIN}-${CustomServer.TIMEOUT_MAX} ms")
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
