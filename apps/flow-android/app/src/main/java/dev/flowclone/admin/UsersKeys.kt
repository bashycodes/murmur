package dev.flowclone.admin

import android.content.Context
import dev.flowclone.core.BlockingCalls
import dev.flowclone.data.BuildCaps
import dev.flowclone.data.DevSettings
import org.json.JSONObject
import java.text.DateFormat
import java.util.Date
import java.util.Locale

/**
 * Settings → Your server → Users & keys (D39), the Android twin of the Windows page
 * (apps/flow-windows/src/server_admin.rs): the owner lists, creates and revokes the server's
 * named keys (D38) when this app's saved token is an admin key. The page and its entry are
 * hidden for a non-admin key (`forbidden` / `whoami.admin == false`) and in the `local`
 * flavour (no network code, no admin natives).
 *
 * Privacy: a created key's secret lives only in the [CreatedKey] handed to the page, which
 * shows it once; it is never logged, persisted or put in `toString()`.
 */
data class WhoAmI(val id: String, val name: String, val admin: Boolean)

/** One key with its usage (`flow_protocol::admin::KeyView`). Times are Unix seconds. */
data class KeyView(
    val id: String,
    val name: String,
    val admin: Boolean = false,
    val created: Long = 0,
    val revoked: Long? = null,
    val lastSeen: Long? = null,
    val client: String? = null,
    val dictations: Long = 0,
    val audioMs: Long = 0,
    val downloads: Long = 0,
    val downloadBytes: Long = 0,
    val current: Boolean = false,
) {
    val active: Boolean get() = revoked == null
}

/** A new key and its secret: the only copy anywhere. Not a data class, so no secret in `toString`. */
class CreatedKey(val key: KeyView, val secret: String) {
    override fun toString() = "CreatedKey(key=$key, secret=<hidden>)"
}

sealed interface AdminResult<out T> {
    data class Ok<T>(val value: T) : AdminResult<T>
    data class Err(val code: String, val message: String = "") : AdminResult<Nothing>
}

/** `flow_protocol::admin::code` values, plus the app's own [NOT_IN_BUILD]. */
object AdminCode {
    const val UNAUTHORIZED = "unauthorized"
    const val FORBIDDEN = "forbidden"
    const val NOT_FOUND = "not_found"
    const val AUTH_OFF = "auth_off"
    const val LAST_ADMIN = "last_admin"
    const val NAME_TAKEN = "name_taken"
    const val BAD_REQUEST = "bad_request"
    const val SERVER_ERROR = "server_error"
    const val CONNECT_FAILED = "connect_failed"
    const val BAD_RESPONSE = "bad_response"
    const val UNSUPPORTED = "unsupported"
    /** The native library has no admin symbols (local flavour, or an old library). */
    const val NOT_IN_BUILD = "not_in_build"
}

/** The admin operations against the saved server. Blocking: call off the main thread. */
interface AdminApi {
    fun whoami(): AdminResult<WhoAmI>
    fun listKeys(): AdminResult<List<KeyView>>
    fun createKey(name: String, admin: Boolean): AdminResult<CreatedKey>
    fun revokeKey(id: String): AdminResult<KeyView>
}

/** Decodes the natives' JSON envelope. */
object AdminJson {
    fun <T> decode(json: String, value: (JSONObject) -> T): AdminResult<T> = try {
        val o = JSONObject(json)
        if (o.optBoolean("ok")) AdminResult.Ok(value(o.getJSONObject("value")))
        else AdminResult.Err(o.optString("code").ifEmpty { AdminCode.BAD_RESPONSE }, o.optString("message"))
    } catch (e: org.json.JSONException) {
        AdminResult.Err(AdminCode.BAD_RESPONSE, "bad answer from the native library")
    }

    fun whoami(o: JSONObject) = WhoAmI(o.optString("id"), o.optString("name"), o.optBoolean("admin"))

    private fun JSONObject.optLongOrNull(k: String): Long? = if (has(k) && !isNull(k)) optLong(k) else null

    fun key(o: JSONObject) = KeyView(
        id = o.optString("id"), name = o.optString("name"), admin = o.optBoolean("admin"),
        created = o.optLong("created"), revoked = o.optLongOrNull("revoked"), lastSeen = o.optLongOrNull("last_seen"),
        client = if (o.has("client") && !o.isNull("client")) o.optString("client") else null,
        dictations = o.optLong("dictations"), audioMs = o.optLong("audio_ms"),
        downloads = o.optLong("downloads"), downloadBytes = o.optLong("download_bytes"), current = o.optBoolean("current"),
    )

    fun keys(o: JSONObject): List<KeyView> {
        val a = o.optJSONArray("keys") ?: return emptyList()
        return (0 until a.length()).map { key(a.getJSONObject(it)) }
    }

    fun created(o: JSONObject) = CreatedKey(key(o.getJSONObject("key")), o.getString("secret"))

    fun revoked(o: JSONObject) = key(o.getJSONObject("key"))
}

/** [AdminApi] over the JNI natives for one server config. */
class NativeAdminApi(private val serverConfigJson: String) : AdminApi {
    private inline fun <T> call(what: String, f: () -> String, noinline value: (JSONObject) -> T): AdminResult<T> {
        BlockingCalls.check(what)
        if (!NativeAdmin.loaded()) return AdminResult.Err(AdminCode.NOT_IN_BUILD, "the speech engine library is missing")
        val json = try { f() } catch (e: UnsatisfiedLinkError) {
            return AdminResult.Err(AdminCode.NOT_IN_BUILD, "this build has no key management")
        }
        return AdminJson.decode(json, value)
    }

    override fun whoami() = call("nativeAdminWhoami", { NativeAdmin.nativeAdminWhoami(serverConfigJson) }, AdminJson::whoami)
    override fun listKeys() = call("nativeAdminListKeys", { NativeAdmin.nativeAdminListKeys(serverConfigJson) }, AdminJson::keys)
    override fun createKey(name: String, admin: Boolean) =
        call("nativeAdminCreateKey", { NativeAdmin.nativeAdminCreateKey(serverConfigJson, name, admin) }, AdminJson::created)
    override fun revokeKey(id: String) = call("nativeAdminRevokeKey", { NativeAdmin.nativeAdminRevokeKey(serverConfigJson, id) }, AdminJson::revoked)
}

/** Page rules and texts (pure, unit-tested). */
object UsersKeys {
    const val TITLE = "Users & keys"

    /** The page can exist at all: a network build with a saved server address and token. */
    fun available(caps: BuildCaps, s: DevSettings) = caps.remote && s.serverConfigured && s.token.isNotEmpty()

    /** Show the entry only for an admin key; any error (403, 401, offline, old server) hides it. */
    fun showEntry(r: AdminResult<WhoAmI>?) = r is AdminResult.Ok && r.value.admin

    /** The page must close: the key is no longer an admin key, or the build can't do this. */
    fun mustLeave(code: String) = code == AdminCode.FORBIDDEN || code == AdminCode.UNAUTHORIZED || code == AdminCode.NOT_IN_BUILD

    /** The client for the saved settings, or null when [available] is false. Tests replace it. */
    @Volatile var factory: (Context) -> AdminApi? = { ctx ->
        val app = dev.flowclone.FlowApp.of(ctx)
        clientFor(app.caps, app.dev.load())
    }

    /** The native client for saved settings [s], or null when [available] is false. */
    fun clientFor(caps: BuildCaps, s: DevSettings): AdminApi? =
        if (available(caps, s)) NativeAdminApi(s.toServerConfigJson().toString()) else null

    /** Text for an error code (mirrors Windows `error_text`). */
    fun errorText(code: String, message: String): String {
        val hint = when (code) {
            AdminCode.FORBIDDEN -> "This app's key is not an admin key."
            AdminCode.UNAUTHORIZED -> "The server no longer accepts this app's key."
            AdminCode.LAST_ADMIN -> "This is the last admin key: create another admin key first."
            AdminCode.NAME_TAKEN -> "A key with this name already exists."
            AdminCode.UNSUPPORTED -> "This server is too old for key management: update flow-server."
            AdminCode.AUTH_OFF -> "This server runs without keys."
            AdminCode.CONNECT_FAILED -> "Can't reach the server."
            AdminCode.NOT_IN_BUILD -> "This build has no key management."
            else -> ""
        }
        return hint.ifEmpty { message.ifEmpty { code }.replaceFirstChar { it.uppercase() } }
    }

    fun errorText(e: AdminResult.Err) = errorText(e.code, e.message)

    /** Local check before a request (the server has the final say on the name's characters). */
    fun nameError(name: String): String? = when {
        name.isBlank() -> "Give the key a name, e.g. phone or laptop."
        name.trim().length > 64 -> "At most 64 characters."
        else -> null
    }

    /** Active keys first (in server order), then revoked ones. */
    fun sorted(keys: List<KeyView>) = keys.filter { it.active } + keys.filter { !it.active }

    /** Revoke is refused for the last active admin key (the server says `last_admin` too). */
    fun canRevoke(k: KeyView, keys: List<KeyView>) = k.active && !(k.admin && keys.count { it.active && it.admin } <= 1)

    fun minutes(ms: Long) = String.format(Locale.US, "%.1f", ms / 60000.0)

    fun megabytes(b: Long) = String.format(Locale.US, "%.1f MB", b / 1e6)

    private fun day(secs: Long) = DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(secs * 1000))

    private fun seen(secs: Long?) = if (secs == null || secs == 0L) "never" else
        DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(secs * 1000))

    /** Row badge: "admin", "this app", both or none. */
    fun badge(k: KeyView) = listOfNotNull("admin".takeIf { k.admin }, "this app".takeIf { k.current }).joinToString(" · ")

    /** Row meta: client, last seen, usage incl. downloads; or when it was revoked. */
    fun meta(k: KeyView): String {
        val usage = "${k.dictations} dictation${if (k.dictations == 1L) "" else "s"} · ${minutes(k.audioMs)} min audio · " +
            "${k.downloads} download${if (k.downloads == 1L) "" else "s"}" + if (k.downloads > 0) " (${megabytes(k.downloadBytes)})" else ""
        val first = (k.client ?: "not used yet") + " · last seen " + seen(k.lastSeen)
        return if (k.revoked != null) "Revoked ${day(k.revoked)} · $usage" else "$first\n$usage · created ${day(k.created)}"
    }

    fun revokeMessage(k: KeyView) =
        "Its next connection is refused. A dictation already running finishes. This can't be undone: to let it back in, create a new key." +
            if (k.current) "\n\nThis is the key this app uses. After revoking it, this app can't reach the server until you save another key in Your server." else ""
}
