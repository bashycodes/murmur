package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * D18 (docs/cloud-stt.md §8.1): one third-party speech-to-text provider, the Kotlin twin of
 * `flow_router::CloudProviderConfig`. API keys are never part of it (they live in
 * [CloudKeyStore], sealed by a per-provider [SecretVault]); this is what goes into the router
 * JSON. `crates/flow-router/fixtures/router_config_cloud.json` is the wire spec (RouterJsonTest).
 */
data class CloudProvider(
    /** `[a-z0-9-]{1,32}`, unique; also names the provider's secure-storage entry. */
    val id: String,
    val preset: String = "custom",
    /** openai_compat | openrouter | elevenlabs | deepgram | assemblyai */
    val adapter: String = "openai_compat",
    /** openai_compat only: "" = auto, whisper, gpt_transcribe. */
    val dialect: String = "",
    val label: String = "",
    val baseUrl: String = "",
    val model: String = "",
    /** "" = follow the dictation languages. */
    val language: String = "",
    val sendDictionary: Boolean = true,
    val sendScreenWords: Boolean = false,
    val zeroRetention: Boolean = true,
    /** "" | wav | flac */
    val formatOverride: String = "",
    val timeoutBaseMs: Long = 6_000,
    /** null = the preset's price. */
    val pricePerHourUsd: Double? = null,
    /** Host the user agreed to send audio to (consent dialog) and when; 0 = no consent. */
    val consentHost: String = "",
    val consentAtMs: Long = 0,
) {
    /** Lower-cased host of [baseUrl] (no port, IPv6 unbracketed), or "". */
    val host: String get() = CloudRules.host(baseUrl) ?: ""

    /** Same rule as the router: consent is void once the base URL points at another host. */
    val consentOk: Boolean get() = consentAtMs > 0 && consentHost.isNotEmpty() && host.isNotEmpty() && consentHost.equals(host, ignoreCase = true)

    val displayName: String get() = label.ifEmpty { id }

    /** The consent dialog's "Send audio to <provider>" button. */
    fun withConsent(nowMs: Long) = copy(consentHost = host, consentAtMs = nowMs)

    fun withoutConsent() = copy(consentHost = "", consentAtMs = 0)

    /**
     * Apply an edit from the provider form: a changed host or adapter voids the consent
     * (the audio would now go somewhere the user did not agree to).
     */
    fun edited(next: CloudProvider): CloudProvider =
        if (!next.host.equals(host, ignoreCase = true) || next.adapter != adapter) next.withoutConsent()
        else next.copy(consentHost = consentHost, consentAtMs = consentAtMs)

    fun toJson(): JSONObject = JSONObject().apply {
        put("id", id); put("preset", preset); put("adapter", adapter); put("dialect", dialect)
        put("label", label); put("base_url", baseUrl); put("model", model); put("language", language)
        put("send_dictionary", sendDictionary); put("send_screen_words", sendScreenWords)
        put("zero_retention", zeroRetention); put("format_override", formatOverride)
        put("timeout_base_ms", timeoutBaseMs)
        put("price_per_hour_usd", pricePerHourUsd ?: JSONObject.NULL)
        put("consent_host", consentHost); put("consent_at_ms", consentAtMs)
    }

    /** No secrets in here, but keep log lines short and uniform. */
    override fun toString() = "CloudProvider($id $adapter $host model=$model consent=$consentOk)"

    companion object {
        /** serde defaults for missing keys, like `#[serde(default)]` on the Rust side. */
        fun fromJson(o: JSONObject) = CloudProvider(
            id = o.optString("id"),
            preset = o.optString("preset", "custom"),
            adapter = o.optString("adapter", "openai_compat"),
            dialect = o.optString("dialect"),
            label = o.optString("label"),
            baseUrl = o.optString("base_url"),
            model = o.optString("model"),
            language = o.optString("language"),
            sendDictionary = o.optBoolean("send_dictionary", true),
            sendScreenWords = o.optBoolean("send_screen_words", false),
            zeroRetention = o.optBoolean("zero_retention", true),
            formatOverride = o.optString("format_override"),
            timeoutBaseMs = o.optLong("timeout_base_ms", 6_000),
            pricePerHourUsd = if (!o.has("price_per_hour_usd") || o.isNull("price_per_hour_usd")) null else o.optDouble("price_per_hour_usd"),
            consentHost = o.optString("consent_host"),
            consentAtMs = o.optLong("consent_at_ms", 0),
        )
    }
}

/** `RouterConfig.cloud` (§8.1). */
data class CloudSettings(
    /** Provider id the cloud route uses; "" = none. */
    val active: String = "",
    val providers: List<CloudProvider> = emptyList(),
    /** Stop using the cloud for the month once this much was spent; null = no budget. */
    val monthlyBudgetUsd: Double? = null,
) {
    val isDefault: Boolean get() = this == CloudSettings()
    val activeProvider: CloudProvider? get() = providers.firstOrNull { it.id == active && active.isNotEmpty() }

    fun provider(id: String) = providers.firstOrNull { it.id == id }

    /** Insert or replace by id (an edit keeps the consent unless host / adapter changed). */
    fun upsert(p: CloudProvider): CloudSettings {
        val old = provider(p.id)
        val next = old?.edited(p) ?: p.withoutConsent()
        val list = if (old == null) providers + next else providers.map { if (it.id == p.id) next else it }
        return copy(providers = list, active = active.ifEmpty { p.id })
    }

    fun remove(id: String): CloudSettings {
        val list = providers.filter { it.id != id }
        return copy(providers = list, active = if (active == id) list.firstOrNull()?.id.orEmpty() else active)
    }

    fun replace(p: CloudProvider) = copy(providers = providers.map { if (it.id == p.id) p else it })

    fun toJson(): JSONObject = JSONObject().apply {
        put("active", active)
        put("providers", JSONArray().apply { providers.forEach { put(it.toJson()) } })
        put("monthly_budget_usd", monthlyBudgetUsd ?: JSONObject.NULL)
    }

    companion object {
        fun fromJson(o: JSONObject?): CloudSettings {
            if (o == null) return CloudSettings()
            val a = o.optJSONArray("providers") ?: JSONArray()
            return CloudSettings(
                active = o.optString("active"),
                providers = (0 until a.length()).mapNotNull { a.optJSONObject(it)?.let(CloudProvider::fromJson) },
                monthlyBudgetUsd = if (!o.has("monthly_budget_usd") || o.isNull("monthly_budget_usd")) null else o.optDouble("monthly_budget_usd"),
            )
        }
    }
}

/**
 * §8.2 presets: the Developer page's "Add provider" list. The native library has the same table
 * (`nativeCloudPresets`); this copy keeps the picker usable (and testable) without it.
 */
object CloudPresets {
    data class Preset(
        val id: String,
        val label: String,
        val adapter: String,
        val baseUrl: String,
        val model: String,
        val dialect: String = "",
        /** Default of the adapter-specific zero-retention flag; null = the adapter has none. */
        val zeroRetention: Boolean?,
        /** What the retention toggle / note says (consent dialog + provider page). */
        val retention: String,
        val keyHint: String,
    ) {
        val editableBaseUrl: Boolean get() = id == "custom"

        fun newProvider(existing: Collection<String>) = CloudProvider(
            id = CloudRules.newId(id, existing), preset = id, adapter = adapter, dialect = dialect, label = label,
            baseUrl = baseUrl, model = model, zeroRetention = zeroRetention ?: false,
        )
    }

    val ALL = listOf(
        Preset("openrouter", "OpenRouter", "openrouter", "https://openrouter.ai/api/v1", "openai/gpt-transcribe", zeroRetention = true,
            retention = "Zero retention: routes only to endpoints that keep no data (provider.zdr). OpenRouter stores request metadata.",
            keyHint = "sk-or-…"),
        Preset("elevenlabs", "ElevenLabs Scribe", "elevenlabs", "https://api.elevenlabs.io", "scribe_v2", zeroRetention = false,
            retention = "Zero retention (enable_logging=false) needs an ElevenLabs enterprise plan; otherwise logs and history are kept.",
            keyHint = "xi-api-key"),
        Preset("groq", "Groq", "openai_compat", "https://api.groq.com/openai/v1", "whisper-large-v3-turbo", zeroRetention = null,
            retention = "Groq keeps data up to 30 days unless you turn on zero data retention in the Groq console (Data Controls).",
            keyHint = "gsk_…"),
        Preset("openai", "OpenAI", "openai_compat", "https://api.openai.com/v1", "gpt-transcribe", dialect = "gpt_transcribe", zeroRetention = null,
            retention = "OpenAI does not train on API data by default; zero retention needs an agreement with OpenAI.",
            keyHint = "sk-…"),
        Preset("deepgram", "Deepgram", "deepgram", "https://api.deepgram.com", "nova-3", zeroRetention = true,
            retention = "Zero retention: opts out of Deepgram's model-improvement programme (mip_opt_out=true).",
            keyHint = "Deepgram API key"),
        Preset("assemblyai", "AssemblyAI", "assemblyai", "https://api.assemblyai.com", "universal", zeroRetention = true,
            retention = "Zero retention: the transcript is deleted right after it is fetched (audio is deleted within 24-48 h).",
            keyHint = "AssemblyAI API key"),
        Preset("custom", "Custom (OpenAI-compatible)", "openai_compat", "https://", "", zeroRetention = null,
            retention = "A custom OpenAI-compatible /v1/audio/transcriptions service: its own retention terms apply (self-hosted speaches / LocalAI keep audio on your machine).",
            keyHint = "API key (optional for self-hosted)"),
    )

    fun of(id: String): Preset = ALL.firstOrNull { it.id == id } ?: ALL.last()
}

/** Kotlin twin of the router's `CloudProviderConfig::validate` / `host` rules (inline form errors). */
object CloudRules {
    private val ID = Regex("^[a-z0-9-]{1,32}$")
    const val MAX_PROVIDERS = 8

    /** Host of an `http(s)://host[:port][/path]` URL, lower-cased; null for anything else. */
    fun host(url: String): String? {
        val u = url.trim()
        val rest = when {
            u.startsWith("https://", ignoreCase = true) -> u.substring(8)
            u.startsWith("http://", ignoreCase = true) -> u.substring(7)
            else -> return null
        }
        val authority = rest.split('/', '?', '#').first()
        if (authority.contains('@')) return null
        val h = if (authority.startsWith("[")) authority.substring(1).substringBefore(']') else authority.substringBefore(':')
        return h.lowercase()
    }

    fun isHttps(url: String) = url.trim().startsWith("https://", ignoreCase = true)

    /** Null when valid, else the first problem (same order as the router). */
    fun validate(p: CloudProvider): String? {
        if (!ID.matches(p.id)) return "provider id must be 1-32 of a-z, 0-9, -"
        if (p.adapter !in ADAPTERS) return "unknown adapter ${p.adapter}"
        val h = host(p.baseUrl) ?: return "base URL must be https://host[/path]"
        if (h.isEmpty()) return "base URL has no host"
        if (!isHttps(p.baseUrl) && !HostRules.plaintextOk(h)) return "public base URL: https:// is required"
        if (p.model.isBlank()) return "model must be set"
        if (p.timeoutBaseMs !in 1_000..60_000) return "timeout must be 1000-60000 ms"
        if (p.pricePerHourUsd != null && !(p.pricePerHourUsd >= 0.0)) return "price must be a non-negative amount"
        return null
    }

    /** `<preset>-<n>`, unique among [existing]. */
    fun newId(preset: String, existing: Collection<String>): String {
        val base = preset.lowercase().filter { it in 'a'..'z' || it in '0'..'9' || it == '-' }.take(28).ifEmpty { "custom" }
        var n = 1
        while ("$base-$n" in existing) n++
        return "$base-$n"
    }

    val ADAPTERS = listOf("openai_compat", "openrouter", "elevenlabs", "deepgram", "assemblyai")
}

/** flow_router `CloudUsage` (this month's counters; persisted by [DevStore.cloudUsageJson]). */
data class CloudUsage(val month: String = "", val providers: List<ProviderUsage> = emptyList()) {
    data class ProviderUsage(
        val providerId: String,
        val requests: Long = 0,
        val billedAudioMs: Long = 0,
        val costUsd: Double = 0.0,
        val costEstimated: Boolean = false,
    )

    val totalCostUsd: Double get() = providers.sumOf { it.costUsd }

    fun of(id: String) = providers.firstOrNull { it.providerId == id }

    companion object {
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
                        )
                    }
                },
            )
        }.getOrDefault(CloudUsage())
    }
}

/** Developer-page / bubble wording for the cloud route (§8.3), pure for the JVM tests. */
object CloudText {
    const val SECTION = "Cloud providers (sends audio to a third party)"
    const val NOT_IN_BUILD = "not in this build"

    fun banner(p: CloudProvider?): String {
        val who = if (p == null) "the provider you choose" else "${p.displayName} (${p.host.ifEmpty { "?" }})"
        return "Off by default. When on, the audio of your dictations leaves this device and is sent to $who, " +
            "along with your dictionary words. It is processed under that provider's terms, not by your server."
    }

    fun consentTitle(p: CloudProvider) = "Send audio to ${p.displayName}?"

    fun consentBody(p: CloudProvider): String =
        banner(p) + "\n\n" + CloudPresets.of(p.preset).retention + "\n\nYou can turn this off at any time."

    fun consentButton(p: CloudProvider) = "Send audio to ${p.displayName}"

    /** `••••abcd` for a stored key; never more than the last 4 characters. */
    fun mask(key: String?): String = when {
        key.isNullOrEmpty() -> "no key"
        key.length <= 4 -> "••••"
        else -> "••••" + key.takeLast(4)
    }

    /** §3.2 failure → short hint (Test provider, fallback toast). */
    fun hint(reason: String?): String? = when (reason) {
        "auth_failed" -> "key rejected"
        "quota_exceeded" -> "out of credits"
        "connect_failed" -> "no internet / base URL wrong"
        "budget_exceeded" -> "monthly budget reached"
        "cloud_not_configured" -> "set a key and accept the consent"
        "busy" -> "provider rate limit, try again"
        "timeout" -> "provider too slow"
        "provider_error" -> "provider error"
        "circuit_open" -> "paused after earlier failures"
        else -> null
    }

    /** Route-failure values that can only come from the cloud route. */
    val CLOUD_ONLY_FAILURES = setOf("cloud_not_configured", "budget_exceeded", "quota_exceeded", "provider_error")

    fun usd(v: Double): String = when {
        v == 0.0 -> "$0"
        v < 0.01 -> "$" + String.format(java.util.Locale.US, "%.4f", v)
        else -> "$" + String.format(java.util.Locale.US, "%.2f", v)
    }

    /** "cloud · Groq · whisper-large-v3-turbo · ≈$0.0012" (Active engine row). */
    fun engineRow(route: dev.flowclone.core.RouteInfo, providerLabel: String?): String {
        val parts = mutableListOf("cloud", providerLabel ?: route.provider ?: "?")
        route.model?.let { parts += it }
        route.costUsd?.let { parts += (if (route.costEstimated == true) "≈" else "") + usd(it) }
        return parts.joinToString(" · ")
    }

    /**
     * User message when the cloud route failed and another engine took over (bubble toast):
     * "Cloud STT failed (key rejected); used on-device". Null when the cloud was not involved.
     */
    fun fallbackMessage(route: dev.flowclone.core.RouteInfo?, cloudWasPrimary: Boolean): String? {
        val f = route?.fallback ?: return null
        if (route.engine == "cloud") return null
        if (f !in CLOUD_ONLY_FAILURES && !cloudWasPrimary) return null
        val used = when (route.engine) { "local" -> "on-device"; "remote" -> "your server"; else -> route.engine }
        return "Cloud STT failed (${hint(f) ?: f}); used $used"
    }

    fun usageLines(u: CloudUsage, s: CloudSettings): List<String> {
        val lines = s.providers.map { p ->
            val x = u.of(p.id)
            if (x == null || x.requests == 0L) "${p.displayName}: no dictations this month"
            else "${p.displayName}: ${x.requests} dictation${if (x.requests == 1L) "" else "s"} · " +
                String.format(java.util.Locale.US, "%.1f min", x.billedAudioMs / 60_000.0) + " · " +
                (if (x.costEstimated) "≈ " else "") + usd(x.costUsd) + (if (x.costEstimated) " (estimated)" else "")
        }
        val budget = s.monthlyBudgetUsd?.let { "Budget ${usd(it)}/month: ${usd(u.totalCostUsd)} used" + if (u.totalCostUsd >= it) " — reached, cloud paused until next month" else "" }
        return listOf("Month ${u.month.ifEmpty { "-" }}") + lines + listOfNotNull(budget)
    }

    /** §3.1 warnings for the route order. */
    fun orderWarning(order: List<DevSettings.Route>, p: CloudProvider?): String? {
        val i = order.indexOf(DevSettings.Route.CLOUD)
        if (i <= 0) return null
        val name = p?.displayName ?: "the cloud provider"
        return when (order[i - 1]) {
            DevSettings.Route.SERVER -> "When your server is unreachable, audio goes to $name."
            DevSettings.Route.CLIENT -> "When no on-device model is loaded, audio goes to $name."
            DevSettings.Route.CLOUD -> null
        }
    }
}

/** The provider page's text inputs → a validated [CloudProvider] (Save / Test stay disabled on error). */
object CloudProviderForm {
    data class Input(
        val base: CloudProvider,
        val label: String,
        val baseUrl: String,
        val model: String,
        val language: String,
        val sendDictionary: Boolean,
        val sendScreenWords: Boolean,
        val zeroRetention: Boolean,
        val timeoutBaseMs: String,
    ) {
        companion object {
            fun of(p: CloudProvider) = Input(
                p, p.label, p.baseUrl, p.model, p.language, p.sendDictionary, p.sendScreenWords, p.zeroRetention, p.timeoutBaseMs.toString(),
            )
        }
    }

    data class Result(val provider: CloudProvider?, val error: String?, val consentVoided: Boolean)

    fun validate(i: Input): Result {
        val t = i.timeoutBaseMs.trim().toLongOrNull()
        val p = i.base.copy(
            label = i.label.trim(), baseUrl = i.baseUrl.trim().trimEnd('/'), model = i.model.trim(), language = i.language.trim().lowercase(),
            sendDictionary = i.sendDictionary, sendScreenWords = i.sendScreenWords, zeroRetention = i.zeroRetention,
            timeoutBaseMs = t ?: -1,
        )
        if (t == null) return Result(null, "timeout must be a whole number of ms (1000-60000)", false)
        if (p.language.isNotEmpty() && !Regex("^[a-z]{2,3}(-[a-z0-9]{2,8})?$").matches(p.language)) {
            return Result(null, "language must be empty (follow dictation languages) or a code like en, de, pt-br", false)
        }
        CloudRules.validate(p)?.let { return Result(null, it, false) }
        val next = i.base.edited(p)
        return Result(next, null, i.base.consentOk && !next.consentOk)
    }
}
