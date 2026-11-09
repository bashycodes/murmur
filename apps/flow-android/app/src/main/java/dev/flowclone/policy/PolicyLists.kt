package dev.flowclone.policy

import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * Which apps get special handling, by package name (and, for the style buckets, by the host of
 * the page open in a browser).
 *
 * The groups are not kept here: they are read from `crates/flow-core/data/app-groups.json`, which
 * the Gradle build packs as a Java resource. flow-core (Rust, both platforms) and the Windows
 * app read the same file, so the apps cannot disagree on a style bucket or a browser, and the
 * shared vectors (`crates/flow-core/fixtures/app-groups/vectors.json`) are checked on both sides.
 *
 * Method (decision D80; the file's `about` lines say the same): each group is defined by what its
 * caller does with it. Membership comes from public sources only: a package name is the id in the
 * app's public Google Play listing (`play.google.com/store/apps/details?id=<package>`), a host is
 * the vendor's own web address. Naming rules cover whole families where the vendor's naming makes
 * that safe (launchers contain "launcher", System UI is a `systemui` part, banks say "bank"); the
 * lists hold the rest. A missing app falls back to the default path (bubble shown, style "other",
 * normal insert checks), except [isRestricted], whose word rule errs toward hiding.
 *
 * Fail-safe: if the file is missing or broken, the error is logged and every app counts as
 * restricted (no bubble, no reads) while every other group is empty, so a bad build hides the
 * bubble instead of crashing the accessibility service or exposing a bank app.
 */
object PolicyLists {
    /** One group: app-id rules and web-host rules (see the JSON's matching rules). */
    class Group(
        val exact: Set<String> = emptySet(),
        val families: List<String> = emptyList(),
        val parts: Set<String> = emptySet(),
        val words: List<String> = emptyList(),
        val hostSuffixes: List<String> = emptyList(),
        val hostPrefixes: List<String> = emptyList(),
    ) {
        /** Exact, a family (`f` or `f.*`), a dot-separated part, or a word anywhere; any case. */
        fun app(id: String?): Boolean {
            val p = id?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return false
            return p in exact || families.any { p == it || p.startsWith("$it.") } ||
                p.split('.').any { it in parts } || words.any { it in p }
        }

        /** [host] (any case, `www.` ignored) equals or is a subdomain of a suffix, or starts with a prefix. */
        fun host(host: String?): Boolean {
            val h = host?.trim()?.lowercase()?.removePrefix("www.")?.takeIf { it.isNotEmpty() } ?: return false
            return hostSuffixes.any { h == it || h.endsWith(".$it") } || hostPrefixes.any { h.startsWith(it) }
        }
    }

    private const val RESOURCE = "/app-groups.json"

    internal class Data(val android: Map<String, Group>, val placeholders: Set<String>, val ok: Boolean)

    /** Groups every caller needs; a file without one of them counts as broken. */
    internal val REQUIRED = listOf(
        "restart_prone", "known_null", "system_search", "in_call", "compose_placeholder_apps",
        "system_ui", "browser", "restricted", "email", "work", "personal",
    )

    /** What [load] returns for a missing or broken file: restricted matches every app. */
    private val FAILED = Data(mapOf("restricted" to Group(words = listOf(""))), emptySet(), ok = false)

    private val data: Data by lazy {
        val text = try {
            PolicyLists::class.java.getResourceAsStream(RESOURCE)?.use { it.readBytes().decodeToString() }
        } catch (e: Exception) {
            null
        }
        load(text)
    }

    /** Parses the JSON text; null or broken text (or a missing group) logs and returns [FAILED]. */
    internal fun load(text: String?): Data {
        val problem = try {
            if (text == null) {
                "$RESOURCE missing from the build"
            } else {
                val root = JSONObject(text)
                val android = root.getJSONObject("android")
                val groups = android.keys().asSequence().associateWith { parse(android.getJSONObject(it)) }
                val missing = REQUIRED.filter { it !in groups }
                if (missing.isEmpty()) {
                    return Data(groups, strings(root.optJSONArray("compose_placeholders")).toSet(), ok = true)
                }
                "$RESOURCE lacks groups $missing"
            }
        } catch (e: Exception) {
            "$RESOURCE is broken: $e"
        }
        try {
            Log.e("PolicyLists", "$problem; treating every app as restricted (D80)")
        } catch (_: RuntimeException) {
            // Plain JVM without Android stubs.
        }
        return FAILED
    }

    /** False when the groups failed to load (see the fail-safe note above). */
    val loaded: Boolean get() = data.ok

    private fun strings(a: JSONArray?): List<String> =
        if (a == null) emptyList() else List(a.length()) { a.getString(it).lowercase() }

    private fun parse(g: JSONObject): Group {
        val apps = g.optJSONObject("apps") ?: JSONObject()
        val hosts = g.optJSONObject("hosts") ?: JSONObject()
        return Group(
            exact = strings(apps.optJSONArray("exact")).toSet(),
            families = strings(apps.optJSONArray("families")),
            parts = strings(apps.optJSONArray("parts")).toSet(),
            words = strings(apps.optJSONArray("words")),
            hostSuffixes = strings(hosts.optJSONArray("suffixes")),
            hostPrefixes = strings(hosts.optJSONArray("prefixes")),
        )
    }

    /** An Android group by its name in the JSON; empty when the groups failed to load. */
    fun group(name: String): Group = group(data, name)

    internal fun group(d: Data, name: String): Group =
        d.android[name] ?: if (d.ok) error("no android app group $name") else EMPTY

    private val EMPTY = Group()

    // ---- Insertion (V3Inserter, FlowAccessibilityService keep-alive) ----

    /** Chat apps whose IME session goes stale: stale-session probe + keep-alive nudge. */
    fun isRestartProne(pkg: String?) = group("restart_prone").app(pkg)

    /** getSurroundingText may be null after a good commit: trust the commit on the first null read. */
    fun isKnownNullSurroundingText(pkg: String?) = group("known_null").app(pkg)

    /** Launchers and Settings search: re-acquire the input surface. */
    fun isSystemSearch(pkg: String?) = group("system_search").app(pkg)

    /** Dialer and in-call screens: re-acquire the input surface. */
    fun isInCall(pkg: String?) = group("in_call").app(pkg)

    /** WhatsApp's compose box shows "Message" as real text until you type; read it as empty. */
    fun usesComposePlaceholders(pkg: String?) = group("compose_placeholder_apps").app(pkg)

    /** The localized "Message" placeholder, normalized (lowercase, no trailing dots). */
    val COMPOSE_PLACEHOLDERS: Set<String> get() = data.placeholders

    // ---- Bubble visibility (BubbleVisibilityPolicy) ----

    /** Notification shade / quick reply: any OEM's System UI package. */
    fun isSystemUi(pkg: String?) = group("system_ui").app(pkg)

    /** Browsers the URL-bar override (IME action Go) applies to. */
    fun isBrowser(pkg: String?) = group("browser").app(pkg)

    /**
     * Banking and payment apps: the bubble stays hidden and nothing is read. Errs toward hiding:
     * any package whose name contains a bank word or a bank-brand word is restricted, plus the
     * listed bank and payment families.
     */
    fun isRestricted(pkg: String?) = group("restricted").app(pkg)

    internal fun isRestricted(d: Data, pkg: String?) = group(d, "restricted").app(pkg)

    // ---- Style buckets (AppCategorizer) ----

    /** Email apps: Gmail, Outlook and any package with a `mail` / `email` part. */
    fun isEmailApp(pkg: String?) = group("email").app(pkg)

    /** Work apps: work chat, meetings, office suites, project tools. */
    fun isWorkApp(pkg: String?) = group("work").app(pkg)

    /** Personal chat and social apps, WhatsApp and Telegram with all their variants. */
    fun isPersonalApp(pkg: String?) = group("personal").app(pkg)

    /** Web email: a `mail.` / `webmail.` host or a listed webmail host. [host] is lowercase without `www.`. */
    fun isEmailHost(host: String?) = group("email").host(host)

    /** Web versions of work tools (Slack, Teams, Google Docs, GitHub, …) and their subdomains. */
    fun isWorkHost(host: String?) = group("work").host(host)

    /** Web versions of personal chat and social apps and their subdomains. */
    fun isPersonalHost(host: String?) = group("personal").host(host)
}
