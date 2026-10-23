package dev.flowclone.data

import org.json.JSONObject

/**
 * Developer settings (docs/hybrid-architecture.md §5): the engine router's configuration.
 * Serialises to flow_router `RouterConfig` JSON — `crates/flow-router/fixtures/router_config_*.json`
 * are the spec (RouterJsonTest). Plain JVM logic; persistence is [DevStore].
 */
data class DevSettings(
    val allowServer: Boolean,
    val allowClient: Boolean,
    /**
     * Engines in try order (§1.3 primary → fallback). The router's `prefer` is derived from it.
     * A list, not a flag, so a later opt-in route (e.g. a user-keyed cloud STT provider, network
     * flavours only) can slot in without reshaping settings, prefs or the UI.
     */
    val routeOrder: List<Route> = DEFAULT_ORDER,
    val host: String = "",
    val port: Int = DEFAULT_PORT,
    val tls: Boolean = false,
    /** 64 lowercase hex chars (colons stripped), or "" for none. */
    val tlsPinSha256: String = "",
    /** Bearer token; never logged, never exported (lives in [TokenVault] at rest). */
    val token: String = "",
    val connectTimeoutMs: Long = DEFAULT_CONNECT_TIMEOUT_MS,
    val commitTimeoutBaseMs: Long = 3_000,
    val healthTtlS: Long = 30,
    val keepLocalWarm: Boolean = true,
    /** (ui) Show S / L on the bubble's preview line, red dot on fallback. */
    val showEngineBadge: Boolean = false,
    /**
     * D18 (docs/cloud-stt.md): third-party cloud STT. Off by default; used only when this is on
     * AND [Route.CLOUD] is in [routeOrder] AND the active provider has consent and a key.
     */
    val allowCloud: Boolean = false,
    /** Providers (no keys: those live in [CloudKeyStore]). */
    val cloud: CloudSettings = CloudSettings(),
) {
    enum class Prefer(val wire: String) { SERVER("server"), CLIENT("client") }

    /** A dictation route. [needsNetwork] routes exist only in flavours with `remote` capability. */
    enum class Route(val wire: String, val needsNetwork: Boolean) {
        SERVER("server", true),
        CLIENT("client", false),
        /** D18: only in builds with `BuildCaps.cloud`; only in the order once the user put it there. */
        CLOUD("cloud", true),
    }

    /** flow_router `prefer`: whichever of server / client comes first in [routeOrder]. */
    val prefer: Prefer
        get() = if (routeOrder.firstOrNull { it == Route.SERVER || it == Route.CLIENT } == Route.CLIENT) Prefer.CLIENT else Prefer.SERVER

    /**
     * Toggles AND build capability (§1.3 "effective flags"); public hosts force TLS (§2.6).
     * A build without the cloud client sends the router no cloud settings at all (C2).
     */
    fun effective(caps: BuildCaps) = copy(
        allowServer = allowServer && caps.remote,
        allowClient = allowClient && caps.local,
        tls = tls || (host.isNotEmpty() && HostRules.classify(host) == HostRules.AddrClass.PUBLIC),
        allowCloud = allowCloud && caps.cloud,
        routeOrder = if (caps.cloud) routeOrder else routeOrder - Route.CLOUD,
        cloud = if (caps.cloud) cloud else CloudSettings(),
    )

    /** The cloud is a route the router may take (it still checks consent, key and budget). */
    val cloudRouted: Boolean get() = allowCloud && Route.CLOUD in routeOrder

    /** The cloud route comes before every other enabled route. */
    val cloudFirst: Boolean
        get() = cloudRouted && routeOrder.firstOrNull { it == Route.CLOUD || (it == Route.SERVER && allowServer) || (it == Route.CLIENT && allowClient) } == Route.CLOUD

    /** No route can dictate ("both off" predates the cloud route; a cloud-only setup is not off). */
    val bothOff: Boolean get() = !allowServer && !allowClient && !cloudRouted
    val serverConfigured: Boolean get() = host.isNotBlank()

    fun toServerConfigJson(): JSONObject = JSONObject().apply {
        put("host", host)
        put("port", port)
        put("tls", tls)
        put("tls_pin_sha256", tlsPinSha256.ifEmpty { null } ?: JSONObject.NULL)
        put("token", token)
        put("connect_timeout_ms", connectTimeoutMs)
        put("commit_timeout_base_ms", commitTimeoutBaseMs)
        put("health_ttl_s", healthTtlS)
    }

    fun toRouterConfigJson(): JSONObject = JSONObject().apply {
        put("allow_server", allowServer)
        put("allow_client", allowClient)
        put("prefer", prefer.wire)
        put("server", toServerConfigJson())
        put("keep_local_warm", keepLocalWarm)
        // D18 fields only once the cloud is touched: pre-cloud fixtures stay byte-identical,
        // like the router's `skip_serializing_if = default`. `order` (when set) replaces `prefer`.
        if (allowCloud || !cloud.isDefault || Route.CLOUD in routeOrder) {
            put("allow_cloud", allowCloud)
            put("cloud", cloud.toJson())
        }
        if (Route.CLOUD in routeOrder) put("order", org.json.JSONArray(routeOrder.map { it.wire }))
    }

    /** What the router gets: effective toggles for this build (a local build never asks for a server). */
    fun routerConfigFor(caps: BuildCaps): String = effective(caps).toRouterConfigJson().toString()

    /** Never print the token. */
    override fun toString(): String =
        "DevSettings(server=$allowServer client=$allowClient prefer=${prefer.wire} host=$host:$port tls=$tls " +
            "pin=${tlsPinSha256.isNotEmpty()} token=${if (token.isEmpty()) "none" else "set"} timeouts=$connectTimeoutMs/$commitTimeoutBaseMs/$healthTtlS warm=$keepLocalWarm " +
            "cloud=$allowCloud order=${routeOrder.joinToString(",") { it.wire }} providers=${cloud.providers.size})"

    companion object {
        /** D24: was 800 ms, too tight through a Tailscale DERP relay (3+ round trips of 150-300 ms). */
        const val DEFAULT_CONNECT_TIMEOUT_MS = 2500L
        const val OLD_DEFAULT_CONNECT_TIMEOUT_MS = 800L

        /** D24: one-time move of an untouched stored 800 ms (the old default) to [DEFAULT_CONNECT_TIMEOUT_MS]. */
        fun migrateConnectTimeout(stored: Long?, migrated: Boolean): Long = when {
            stored == null -> DEFAULT_CONNECT_TIMEOUT_MS
            !migrated && stored == OLD_DEFAULT_CONNECT_TIMEOUT_MS -> DEFAULT_CONNECT_TIMEOUT_MS
            else -> stored
        }

        const val DEFAULT_PORT = 7313
        const val MIN_TOKEN_LEN = 16
        val DEFAULT_ORDER = listOf(Route.SERVER, Route.CLIENT)

        fun orderFor(p: Prefer) = if (p == Prefer.CLIENT) listOf(Route.CLIENT, Route.SERVER) else DEFAULT_ORDER

        /**
         * Stored form `server,client[,cloud]`; unknown names (from a newer build) are skipped, missing
         * server / client appended. The cloud is never appended: it is in the order only when the
         * user put it there (enable flow, §3.1 step 4).
         */
        fun parseOrder(csv: String?): List<Route> {
            val named = csv.orEmpty().split(',').mapNotNull { n -> Route.entries.firstOrNull { it.wire == n.trim() } }.distinct()
            return named + Route.entries.filter { it !in named && it != Route.CLOUD }
        }

        /** §3.1 step 4: enabling the cloud puts it first, keeping the previous server / client order. */
        fun withCloudFirst(order: List<Route>) = listOf(Route.CLOUD) + (order - Route.CLOUD)

        /** §5 defaults: server on in hybrid/remote builds, client on in local/hybrid builds. */
        fun defaults(caps: BuildCaps) = DevSettings(allowServer = caps.remote, allowClient = caps.local)

        /** Parses `RouterConfig` JSON (serde defaults for missing keys, like `#[serde(default)]`). */
        fun fromRouterConfigJson(json: String, caps: BuildCaps = BuildCaps.ALL): DevSettings {
            val o = JSONObject(json)
            val d = defaults(caps)
            val s = o.optJSONObject("server") ?: JSONObject()
            val order = o.optJSONArray("order")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
            return DevSettings(
                allowServer = o.optBoolean("allow_server", d.allowServer),
                allowClient = o.optBoolean("allow_client", d.allowClient),
                routeOrder = if (order.isNotEmpty()) parseOrder(order.joinToString(","))
                else orderFor(Prefer.entries.firstOrNull { it.wire == o.optString("prefer") } ?: Prefer.SERVER),
                host = s.optString("host", ""),
                port = s.optInt("port", DEFAULT_PORT),
                tls = s.optBoolean("tls", false),
                tlsPinSha256 = if (s.isNull("tls_pin_sha256")) "" else s.optString("tls_pin_sha256"),
                token = s.optString("token", ""),
                connectTimeoutMs = s.optLong("connect_timeout_ms", DEFAULT_CONNECT_TIMEOUT_MS),
                commitTimeoutBaseMs = s.optLong("commit_timeout_base_ms", 3_000),
                healthTtlS = s.optLong("health_ttl_s", 30),
                keepLocalWarm = o.optBoolean("keep_local_warm", true),
                allowCloud = o.optBoolean("allow_cloud", false),
                cloud = CloudSettings.fromJson(o.optJSONObject("cloud")),
            )
        }
    }
}

/**
 * This APK's flavour capabilities (`BuildConfig.CAP_LOCAL` / `CAP_REMOTE` / `CAP_CLOUD`).
 * [cloud] (D18): the native library has the third-party STT client (hybrid / remote).
 */
data class BuildCaps(val local: Boolean, val remote: Boolean, val cloud: Boolean = remote) {
    val flavor: String get() = when {
        local && remote -> "hybrid"
        remote -> "remote"
        else -> "local"
    }

    companion object {
        val ALL = BuildCaps(local = true, remote = true)
        val LOCAL = BuildCaps(local = true, remote = false, cloud = false)
        val REMOTE = BuildCaps(local = false, remote = true)
        fun current() = BuildCaps(dev.flowclone.BuildConfig.CAP_LOCAL, dev.flowclone.BuildConfig.CAP_REMOTE, dev.flowclone.BuildConfig.CAP_CLOUD)
    }
}

/**
 * Kotlin twin of `flow_router::net` (validate_host / classify, §2.6 + §5). The router validates
 * again; this one drives inline errors and the TLS lock before anything reaches native code.
 */
object HostRules {
    enum class AddrClass { LOOPBACK, LAN, TAILNET, LOCAL_NAME, PUBLIC }

    fun bare(host: String): String {
        val h = host.trim()
        return if (h.startsWith("[") && h.endsWith("]")) h.substring(1, h.length - 1) else h
    }

    /** Normalised host (trimmed; IPv6 without brackets), or an error message. */
    fun validate(host: String): Result<String> {
        val h = host.trim()
        fun err(m: String) = Result.failure<String>(IllegalArgumentException(m))
        if (h.isEmpty()) return err("host is empty")
        if (h.any { it.isWhitespace() }) return err("host must not contain whitespace")
        if (h.contains("://") || h.contains('/')) return err("enter the host only (no scheme or path)")
        val b = bare(h)
        if (b.contains(':')) return if (parseIpv6(b) != null) Result.success(b) else err("not a valid IPv6 address (and ports go in the port field)")
        if (b != h) return err("brackets are only for IPv6 addresses")
        val parts = h.split('.')
        if (parts.size == 4 && parts.all { p -> p.isNotEmpty() && p.all { it in '0'..'9' } }) {
            for (p in parts) {
                if (p.length > 1 && p.startsWith('0')) return err("IPv4 parts must not have leading zeros")
                if (p.length > 3 || p.toInt() > 255) return err("IPv4 parts must be 0-255")
            }
            return Result.success(h)
        }
        if (h.length > 253) return err("hostname longer than 253 characters")
        val name = h.removeSuffix(".")
        val labels = name.split('.')
        for (l in labels) {
            if (l.isEmpty() || l.length > 63) return err("hostname labels must be 1-63 characters")
            if (!l.all { it.isAsciiAlnum() || it == '-' }) return err("hostnames may contain only letters, digits, '-' and '.'")
            if (l.startsWith('-') || l.endsWith('-')) return err("hostname labels must not start or end with '-'")
        }
        if (labels.all { l -> l.all { it in '0'..'9' } }) return err("not a valid IPv4 address")
        return Result.success(h)
    }

    fun classify(host: String): AddrClass {
        val h = bare(host)
        parseIpv4(h)?.let { return classifyV4(it) }
        parseIpv6(h)?.let { return classifyV6(it) }
        val name = h.trimEnd('.').lowercase()
        return when {
            name == "localhost" || name.endsWith(".localhost") -> AddrClass.LOOPBACK
            name.endsWith(".ts.net") -> AddrClass.TAILNET
            name.endsWith(".local") || !name.contains('.') -> AddrClass.LOCAL_NAME
            else -> AddrClass.PUBLIC
        }
    }

    fun plaintextOk(host: String) = classify(host) != AddrClass.PUBLIC

    private fun Char.isAsciiAlnum() = this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9'

    private fun classifyV4(o: IntArray): AddrClass = when {
        o[0] == 127 -> AddrClass.LOOPBACK
        o[0] == 100 && (o[1] and 0xC0) == 64 -> AddrClass.TAILNET
        o[0] == 10 || (o[0] == 172 && o[1] in 16..31) || (o[0] == 192 && o[1] == 168) -> AddrClass.LAN
        o[0] == 169 && o[1] == 254 -> AddrClass.LAN
        else -> AddrClass.PUBLIC
    }

    private fun classifyV6(s: IntArray): AddrClass {
        // ::ffff:a.b.c.d (IPv4-mapped) classifies as the IPv4 address.
        if ((0 until 5).all { s[it] == 0 } && s[5] == 0xffff) {
            return classifyV4(intArrayOf(s[6] shr 8, s[6] and 0xff, s[7] shr 8, s[7] and 0xff))
        }
        return when {
            (0 until 7).all { s[it] == 0 } && s[7] == 1 -> AddrClass.LOOPBACK
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0 -> AddrClass.TAILNET
            (s[0] and 0xfe00) == 0xfc00 || (s[0] and 0xffc0) == 0xfe80 -> AddrClass.LAN
            else -> AddrClass.PUBLIC
        }
    }

    /** Strict dotted quad (as Rust's `Ipv4Addr::from_str`: no leading zeros), or null. */
    fun parseIpv4(s: String): IntArray? {
        val p = s.split('.')
        if (p.size != 4) return null
        return IntArray(4) { i ->
            val x = p[i]
            if (x.isEmpty() || x.length > 3 || !x.all { it in '0'..'9' } || (x.length > 1 && x[0] == '0')) return null
            x.toInt().takeIf { it <= 255 } ?: return null
        }
    }

    /** RFC 4291 text form (`::` compression, trailing dotted quad), as Rust's `Ipv6Addr::from_str`; no zone ids. */
    fun parseIpv6(s: String): IntArray? {
        if (s.isEmpty() || s.count { it == ':' } < 2) return null
        val dbl = s.indexOf("::")
        if (dbl >= 0 && s.indexOf("::", dbl + 1) >= 0) return null
        fun groups(part: String, allowV4: Boolean): List<Int>? {
            if (part.isEmpty()) return emptyList()
            val g = part.split(':')
            val out = ArrayList<Int>()
            for ((i, x) in g.withIndex()) {
                if (i == g.lastIndex && allowV4 && x.contains('.')) {
                    val v4 = parseIpv4(x) ?: return null
                    out += (v4[0] shl 8) or v4[1]; out += (v4[2] shl 8) or v4[3]
                    continue
                }
                if (x.isEmpty() || x.length > 4 || !x.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return null
                out += x.toInt(16)
            }
            return out
        }
        return if (dbl < 0) {
            groups(s, true)?.takeIf { it.size == 8 }?.toIntArray()
        } else {
            val head = groups(s.substring(0, dbl), false) ?: return null
            val tail = groups(s.substring(dbl + 2), true) ?: return null
            if (head.size + tail.size > 7) return null
            (head + List(8 - head.size - tail.size) { 0 } + tail).toIntArray()
        }
    }
}

/**
 * The Developer page's raw text inputs → validated [DevSettings] (§5 table). Save stays
 * disabled while [Result.errors] is non-empty; [Result.warnings] never block saving.
 */
object DevSettingsForm {
    enum class Field { HOST, PORT, PIN, TOKEN, CONNECT_TIMEOUT, COMMIT_TIMEOUT, HEALTH_TTL, CLOUD_BUDGET }

    data class Input(
        val allowServer: Boolean,
        val allowClient: Boolean,
        val routeOrder: List<DevSettings.Route>,
        val host: String,
        val port: String,
        val tls: Boolean,
        val pin: String,
        val token: String,
        val connectTimeoutMs: String,
        val commitTimeoutBaseMs: String,
        val healthTtlS: String,
        val keepLocalWarm: Boolean,
        val showEngineBadge: Boolean,
        val allowCloud: Boolean = false,
        /** Providers + active id as stored (edited on the provider page); budget comes from [cloudBudget]. */
        val cloud: CloudSettings = CloudSettings(),
        /** Monthly budget in USD; blank = none. */
        val cloudBudget: String = "",
    ) {
        companion object {
            fun of(s: DevSettings) = Input(
                s.allowServer, s.allowClient, s.routeOrder, s.host, s.port.toString(), s.tls, s.tlsPinSha256, s.token,
                s.connectTimeoutMs.toString(), s.commitTimeoutBaseMs.toString(), s.healthTtlS.toString(), s.keepLocalWarm, s.showEngineBadge,
                s.allowCloud, s.cloud, s.cloud.monthlyBudgetUsd?.let { budgetText(it) }.orEmpty(),
            )

            fun budgetText(v: Double): String = if (v == Math.floor(v)) v.toLong().toString() else v.toString()
        }
    }

    data class Result(
        val settings: DevSettings?,
        val errors: Map<Field, String>,
        val warnings: List<String>,
        /** TLS forced on and locked: the host is a public address (§2.6). */
        val tlsLocked: Boolean,
    ) {
        val canSave: Boolean get() = errors.isEmpty() && settings != null
    }

    const val BOTH_OFF = "Dictation disabled: turn on Allow server or Allow client."
    const val NO_HOST = "Server enabled but no host set: dictation will use the client (or fail if the client is off)."
    const val PLAINTEXT = "Unencrypted (ws://): OK on Tailscale (WireGuard), LAN only otherwise."
    const val CLOUD_NO_PROVIDER = "Cloud allowed but no provider is active: add one under Cloud providers."
    const val CLOUD_NO_CONSENT = "The active cloud provider has no consent for its current host: it will be skipped."
    const val CLOUD_NOT_IN_ORDER = "Cloud allowed but not in the route order: it will not be used."
    const val NO_TOKEN = "No token: a server with auth (the default) will reject the connection."

    fun validate(i: Input, caps: BuildCaps): Result {
        val errors = LinkedHashMap<Field, String>()
        val warnings = ArrayList<String>()
        val hostRaw = i.host.trim()
        var host = ""
        if (hostRaw.isNotEmpty()) {
            HostRules.validate(hostRaw).onSuccess { host = it }.onFailure { errors[Field.HOST] = it.message ?: "invalid host" }
        }
        fun num(f: Field, raw: String, lo: Long, hi: Long, label: String): Long? {
            val v = raw.trim().toLongOrNull()
            if (v == null || v !in lo..hi) { errors[f] = "$label must be a whole number $lo-$hi"; return null }
            return v
        }
        val port = num(Field.PORT, i.port, 1, 65_535, "Port")
        val pin = i.pin.trim().replace(":", "").lowercase()
        if (pin.isNotEmpty() && (pin.length != 64 || !pin.all { it in '0'..'9' || it in 'a'..'f' })) {
            errors[Field.PIN] = "Certificate pin must be 64 hex characters (SHA-256 of the leaf certificate)"
        }
        val token = i.token.trim()
        if (token.isNotEmpty() && token.length < DevSettings.MIN_TOKEN_LEN) errors[Field.TOKEN] = "Token must be at least ${DevSettings.MIN_TOKEN_LEN} characters"
        val ct = num(Field.CONNECT_TIMEOUT, i.connectTimeoutMs, 100, 10_000, "Connect timeout (ms)")
        val cb = num(Field.COMMIT_TIMEOUT, i.commitTimeoutBaseMs, 500, 60_000, "Commit timeout (ms)")
        val ttl = num(Field.HEALTH_TTL, i.healthTtlS, 5, 600, "Health cache (s)")
        val tlsLocked = host.isNotEmpty() && HostRules.classify(host) == HostRules.AddrClass.PUBLIC
        val tls = i.tls || tlsLocked
        val allowServer = i.allowServer && caps.remote
        val allowClient = i.allowClient && caps.local
        val allowCloud = i.allowCloud && caps.cloud
        val budgetRaw = i.cloudBudget.trim().removePrefix("$").trim()
        val budget = if (budgetRaw.isEmpty()) null else budgetRaw.toDoubleOrNull()?.takeIf { it >= 0.0 && it.isFinite() }
        if (budgetRaw.isNotEmpty() && budget == null) errors[Field.CLOUD_BUDGET] = "Budget must be an amount in USD (e.g. 5 or 2.50), or empty for none"
        val order = if (caps.cloud) i.routeOrder else i.routeOrder - DevSettings.Route.CLOUD
        val cloudRouted = allowCloud && DevSettings.Route.CLOUD in order
        if (!allowServer && !allowClient && !cloudRouted) warnings += BOTH_OFF
        if (allowCloud) {
            val p = i.cloud.activeProvider
            when {
                p == null -> warnings += CLOUD_NO_PROVIDER
                !p.consentOk -> warnings += CLOUD_NO_CONSENT
            }
            if (DevSettings.Route.CLOUD !in order) warnings += CLOUD_NOT_IN_ORDER
            CloudText.orderWarning(order, p)?.let { warnings += it }
        }
        if (allowServer && host.isEmpty() && Field.HOST !in errors) warnings += NO_HOST
        if (allowServer && host.isNotEmpty() && !tls) warnings += PLAINTEXT
        if (allowServer && host.isNotEmpty() && token.isEmpty()) warnings += NO_TOKEN
        val settings = if (errors.isEmpty()) DevSettings(
            allowServer = allowServer, allowClient = allowClient, host = host, port = port!!.toInt(),
            tls = tls, tlsPinSha256 = if (tls) pin else "", token = token, connectTimeoutMs = ct!!, commitTimeoutBaseMs = cb!!,
            healthTtlS = ttl!!, keepLocalWarm = i.keepLocalWarm, showEngineBadge = i.showEngineBadge,
            // A build without the cloud keeps the stored providers / order untouched (prefs survive a
            // flavour switch); effective() strips them from what the router sees.
            allowCloud = allowCloud, routeOrder = i.routeOrder,
            cloud = if (caps.cloud) i.cloud.copy(monthlyBudgetUsd = budget) else i.cloud,
        ) else null
        return Result(settings, errors, warnings, tlsLocked)
    }
}

/** §5 "Access": visible in builds with `remote`; client-only builds unlock with 7 taps on the version row. */
class DevUnlock(private val now: () -> Long, private val taps: Int = TAPS, private val gapMs: Long = 3_000) {
    private var count = 0
    private var last = Long.MIN_VALUE / 2

    /** Returns true on the tap that unlocks. */
    fun tap(): Boolean {
        val t = now()
        count = if (t - last <= gapMs) count + 1 else 1
        last = t
        if (count >= taps) { count = 0; return true }
        return false
    }

    /** Taps still needed (for "n more taps" toasts after the 3rd). */
    val remaining: Int get() = taps - count

    companion object {
        const val TAPS = 7
        fun visible(caps: BuildCaps, unlocked: Boolean) = caps.remote || unlocked
    }
}

/** The last commit, for the Developer page's "Active engine" row and the bubble badge. In-memory only. */
data class LastDictation(val result: dev.flowclone.core.CommitResult, val atMs: Long) {
    companion object {
        fun of(r: dev.flowclone.core.CommitResult, atMs: Long) = LastDictation(r, atMs)
    }
}
