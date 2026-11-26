package dev.murmur.admin

import android.content.Context
import dev.murmur.core.BlockingCalls
import dev.murmur.core.DictEntry
import dev.murmur.data.BuildCaps
import dev.murmur.data.DevSettings
import org.json.JSONObject
import java.io.File
import java.util.concurrent.CopyOnWriteArraySet
import kotlin.concurrent.thread

/**
 * The team dictionary: the server's shared word list (`murmur_protocol::dictionary`). Any key
 * reads it, admin keys edit it (Users & keys). The app merges it under the personal dictionary
 * ([merge]) into `SessionConfig.dictionary` before a session, so every route, the hotwords, the
 * prompt, cleanup terms and replacements get it. It is never written into the personal
 * [dev.murmur.data.DictionaryStore], backups or exports. Cache, ETag and offline: Rust
 * (`murmur_remote::team_dict`); JNI_CONTRACT.md "Team dictionary".
 */
object TeamDictionary {
    const val CACHE_FILE = "team-dictionary.json"
    const val TOO_MANY = "too_many"

    /**
     * `murmur_core::dictionary::with_team`: [personal] as is, then the [team] entries that are not
     * snippets, not blank and whose word is not already there (case and spacing ignored); team
     * word and replacement trimmed, a blank replacement dropped.
     */
    fun merge(personal: List<DictEntry>, team: List<DictEntry>): List<DictEntry> {
        if (team.isEmpty()) return personal
        val seen = personal.mapTo(HashSet()) { key(it.word) }
        val out = personal.toMutableList()
        for (e in team) {
            if (e.isSnippet) continue
            val k = key(e.word)
            if (k.isEmpty() || !seen.add(k)) continue
            out += DictEntry(e.word.trim(), e.replacement?.trim()?.takeIf { it.isNotEmpty() })
        }
        return out
    }

    /** `murmur_core::postprocess::word_key`: trimmed, simple case fold, whitespace runs = one space. */
    fun key(word: String): String {
        val out = StringBuilder()
        var space = false
        word.trim().codePoints().forEach { c ->
            if (Character.isWhitespace(c) || Character.isSpaceChar(c)) {
                if (!space && out.isNotEmpty()) out.append(' ')
                space = true
            } else {
                space = false
                out.appendCodePoint(fold(c))
            }
        }
        return out.toString()
    }

    private fun fold(c: Int): Int = when (c) {
        'ς'.code -> 'σ'.code
        'ſ'.code -> 's'.code
        'ϐ'.code -> 'β'.code
        'ϑ'.code -> 'θ'.code
        'ϕ'.code -> 'φ'.code
        'ϖ'.code -> 'π'.code
        'ϰ'.code -> 'κ'.code
        'ϱ'.code -> 'ρ'.code
        'ϵ'.code -> 'ε'.code
        'ẛ'.code -> 'ṡ'.code
        else -> Character.toLowerCase(c)
    }

    /** `{entries:[{word, replacement?}]}` → entries (a team entry is never a snippet). */
    fun entries(o: JSONObject): List<DictEntry> {
        val a = o.optJSONArray("entries") ?: return emptyList()
        return (0 until a.length()).mapNotNull { i ->
            val e = a.optJSONObject(i) ?: return@mapNotNull null
            val word = e.optString("word").trim()
            if (word.isEmpty()) return@mapNotNull null
            val r = if (e.has("replacement") && !e.isNull("replacement")) e.optString("replacement").trim().ifEmpty { null } else null
            DictEntry(word, r)
        }
    }

    fun refreshed(o: JSONObject) = TeamRefresh(
        entries(o), o.optBoolean("changed"),
        o.optJSONObject("error")?.let { AdminResult.Err(it.optString("code").ifEmpty { AdminCode.BAD_RESPONSE }, it.optString("message")) },
    )

    /** A network build with a saved server address and key (as on Windows; any key reads). */
    fun available(caps: BuildCaps, s: DevSettings) = caps.remote && s.serverConfigured && s.token.isNotBlank()

    /** The client for the saved settings, or null without a server. Tests replace it. */
    @Volatile var factory: (Context) -> TeamDictionaryApi? = { ctx ->
        val app = dev.murmur.MurmurApp.of(ctx)
        clientFor(app.caps, app.dev.load(), File(ctx.filesDir, CACHE_FILE))
    }

    fun clientFor(caps: BuildCaps, s: DevSettings, cache: File): TeamDictionaryApi? =
        if (available(caps, s)) NativeTeamDictionaryApi(s.toServerConfigJson().toString(), cache.path) else null

    /**
     * Local check before an add (the server has the final say). Characters are code points, as
     * in `murmur_protocol::dictionary::check_entry` (an emoji or other astral character is one).
     */
    fun wordError(word: String): String? = when {
        word.isBlank() -> "Type the word."
        word.trim().let { it.codePointCount(0, it.length) } > 64 -> "At most 64 characters."
        else -> null
    }

    /** Text for a failed edit: the server's message for a bad entry, else the Users & keys text. */
    fun errorText(e: AdminResult.Err): String = when (e.code) {
        AdminCode.FORBIDDEN -> "Only an admin key can change the team dictionary."
        TOO_MANY -> e.message.ifEmpty { "The team dictionary is full." }.replaceFirstChar { it.uppercase() }
        AdminCode.BAD_REQUEST -> e.message.ifEmpty { "The server refused this entry." }.replaceFirstChar { it.uppercase() }
        AdminCode.NOT_FOUND -> "That word is no longer in the team dictionary."
        AdminCode.UNSUPPORTED -> "This server is too old for a team dictionary: update murmur-server."
        else -> UsersKeys.errorText(e)
    }
}

/** A refresh: [entries] is the list to use (fresh, the cache, or empty); [error] says why it is not fresh. */
data class TeamRefresh(val entries: List<DictEntry>, val changed: Boolean, val error: AdminResult.Err? = null)

/** The team dictionary against the saved server. All but [cached] are blocking: call off the main thread. */
interface TeamDictionaryApi {
    /** Which server and key this client talks to: an answer is used only while it is still the saved one. */
    val source: String get() = ""
    /** The cached list for this server; no network. */
    fun cached(): AdminResult<List<DictEntry>>
    fun refresh(): AdminResult<TeamRefresh>
    fun add(word: String, replacement: String): AdminResult<List<DictEntry>>
    fun remove(word: String): AdminResult<List<DictEntry>>
}

/** [TeamDictionaryApi] over the JNI natives; `local` (no symbols) answers [AdminCode.NOT_IN_BUILD]. */
class NativeTeamDictionaryApi(private val serverConfigJson: String, private val cachePath: String) : TeamDictionaryApi {
    // In memory only, never logged: address, port and key.
    override val source: String get() = serverConfigJson

    private inline fun <T> call(what: String?, f: () -> String, noinline value: (JSONObject) -> T): AdminResult<T> {
        what?.let(BlockingCalls::check)
        if (!NativeAdmin.loaded()) return AdminResult.Err(AdminCode.NOT_IN_BUILD, "the speech engine library is missing")
        val json = try { f() } catch (e: UnsatisfiedLinkError) {
            return AdminResult.Err(AdminCode.NOT_IN_BUILD, "this build has no team dictionary")
        }
        return AdminJson.decode(json, value)
    }

    override fun cached() = call(null, { NativeAdmin.nativeTeamDictionaryCached(serverConfigJson, cachePath) }, TeamDictionary::entries)
    override fun refresh() =
        call("nativeTeamDictionaryRefresh", { NativeAdmin.nativeTeamDictionaryRefresh(serverConfigJson, cachePath) }, TeamDictionary::refreshed)
    override fun add(word: String, replacement: String) =
        call("nativeTeamDictionaryAdd", { NativeAdmin.nativeTeamDictionaryAdd(serverConfigJson, cachePath, word, replacement) }, TeamDictionary::entries)
    override fun remove(word: String) =
        call("nativeTeamDictionaryRemove", { NativeAdmin.nativeTeamDictionaryRemove(serverConfigJson, cachePath, word) }, TeamDictionary::entries)
}

/**
 * The current team list, in memory. [load] reads the cache (app start, no network); [refresh]
 * asks the server on a worker thread; [add] / [remove] are the admin edits (blocking). Without a
 * server ([client] null) the list is empty and nothing is fetched. [onChange] runs after the
 * list changed (D68: the engine re-plans its Parakeet search); [listeners] let open pages redraw.
 * An answer is used only if the saved server and key are still the ones it was asked with
 * ([TeamDictionaryApi.source]): a slow answer from the old server never lands.
 */
class TeamDictionaryStore(private val client: () -> TeamDictionaryApi?) {
    @Volatile var entries: List<DictEntry> = emptyList(); private set
    @Volatile var onChange: (() -> Unit)? = null
    private val listeners = CopyOnWriteArraySet<() -> Unit>()

    fun addListener(l: () -> Unit) { listeners += l }
    fun removeListener(l: () -> Unit) { listeners -= l }

    /** [personal] with the team list under it ([TeamDictionary.merge]). */
    fun merged(personal: List<DictEntry>): List<DictEntry> = TeamDictionary.merge(personal, entries)

    /** The cached list for the saved server (file I/O only). */
    fun load() {
        val c = client()
        set(if (c == null) emptyList() else (c.cached() as? AdminResult.Ok)?.value ?: emptyList())
    }

    /** Blocking: asks the server, keeps what it answers (the cache when offline). Null without a server. */
    fun refreshNow(): TeamRefresh? {
        val c = client() ?: run { set(emptyList()); return null }
        return when (val r = c.refresh()) {
            is AdminResult.Ok -> r.value.also { setFor(c, it.entries) }
            // Only a bad config (or a build without the natives): keep what we have.
            is AdminResult.Err -> TeamRefresh(entries, false, r)
        }
    }

    /** [refreshNow] on a worker thread; [done] runs there too. */
    fun refresh(done: ((TeamRefresh?) -> Unit)? = null) {
        thread(name = "team-dictionary", isDaemon = true) {
            val r = runCatching { refreshNow() }.getOrNull()
            done?.invoke(r)
        }
    }

    /** Blocking admin edit; the answer's list is the new one. */
    fun add(word: String, replacement: String): AdminResult<List<DictEntry>> = edit { it.add(word.trim(), replacement.trim()) }

    fun remove(word: String): AdminResult<List<DictEntry>> = edit { it.remove(word) }

    private fun edit(f: (TeamDictionaryApi) -> AdminResult<List<DictEntry>>): AdminResult<List<DictEntry>> {
        val c = client() ?: return AdminResult.Err(AdminCode.NOT_IN_BUILD, "no server")
        return f(c).also { if (it is AdminResult.Ok) setFor(c, it.value) }
    }

    /** [set] if [c] still talks to the saved server with the saved key; else drop [new]. */
    @Synchronized private fun setFor(c: TeamDictionaryApi, new: List<DictEntry>) {
        if (client()?.source == c.source) set(new)
    }

    @Synchronized private fun set(new: List<DictEntry>) {
        if (new == entries) return
        entries = new
        onChange?.invoke()
        for (l in listeners) l()
    }
}
