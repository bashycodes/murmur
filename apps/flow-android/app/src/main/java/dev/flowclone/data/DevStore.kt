package dev.flowclone.data

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The few SharedPreferences operations [DevStore] needs (an in-memory twin backs the JVM tests). */
interface KeyValue {
    fun getString(k: String): String?
    fun getBoolean(k: String, d: Boolean): Boolean
    fun getLong(k: String, d: Long): Long
    fun contains(k: String): Boolean
    fun edit(block: Editor.() -> Unit)

    interface Editor {
        fun putString(k: String, v: String?)
        fun putBoolean(k: String, v: Boolean)
        fun putLong(k: String, v: Long)
        fun remove(k: String)
        fun clear()
    }
}

class PrefsKeyValue(private val sp: SharedPreferences) : KeyValue {
    override fun getString(k: String) = sp.getString(k, null)
    override fun getBoolean(k: String, d: Boolean) = sp.getBoolean(k, d)
    override fun getLong(k: String, d: Long) = sp.getLong(k, d)
    override fun contains(k: String) = sp.contains(k)
    override fun edit(block: KeyValue.Editor.() -> Unit) {
        val e = sp.edit()
        object : KeyValue.Editor {
            override fun putString(k: String, v: String?) { e.putString(k, v) }
            override fun putBoolean(k: String, v: Boolean) { e.putBoolean(k, v) }
            override fun putLong(k: String, v: Long) { e.putLong(k, v) }
            override fun remove(k: String) { e.remove(k) }
            override fun clear() { e.clear() }
        }.block()
        e.commit() // the router must see the new config on the very next session
    }
}

/** Encrypts secrets at rest (the server token today; any future per-provider API key goes through the same box). */
interface SecretBox {
    fun seal(plain: String): String
    /** Null when the ciphertext can't be opened (key lost after a restore, tampering). */
    fun open(sealed: String): String?
}

/** §2.5: Android Keystore AES-GCM key `flow_dev_token`; only ciphertext ("iv:ct", base64) is stored. */
object TokenVault : SecretBox {
    private const val ALIAS = "flow_dev_token"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
        }.generateKey()
    }

    override fun seal(plain: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        val ct = c.doFinal(plain.toByteArray(Charsets.UTF_8))
        return Base64.encodeToString(c.iv, Base64.NO_WRAP) + ":" + Base64.encodeToString(ct, Base64.NO_WRAP)
    }

    override fun open(sealed: String): String? = runCatching {
        val (iv, ct) = sealed.split(':', limit = 2).map { Base64.decode(it, Base64.NO_WRAP) }
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv)) }
        String(c.doFinal(ct), Charsets.UTF_8)
    }.getOrNull()
}

/**
 * Developer settings at rest: SharedPreferences file `dev` (backup is disabled app-wide).
 * Keys mirror `RouterConfig` minus the token, which is stored sealed by [SecretBox].
 * A value that is missing (fresh install / new flavour) falls back to the flavour's defaults.
 */
class DevStore(private val kv: KeyValue, private val caps: BuildCaps, private val box: SecretBox) {
    constructor(context: Context, caps: BuildCaps = BuildCaps.current()) :
        this(PrefsKeyValue(context.getSharedPreferences(FILE, Context.MODE_PRIVATE)), caps, TokenVault)

    fun load(): DevSettings {
        val d = DevSettings.defaults(caps)
        val stored = DevSettings(
            allowServer = kv.getBoolean("allow_server", d.allowServer),
            allowClient = kv.getBoolean("allow_client", d.allowClient),
            routeOrder = kv.getString("route_order")?.let(DevSettings::parseOrder) ?: d.routeOrder,
            host = kv.getString("server.host") ?: d.host,
            port = kv.getLong("server.port", d.port.toLong()).toInt(),
            tls = kv.getBoolean("server.tls", d.tls),
            tlsPinSha256 = kv.getString("server.tls_pin_sha256") ?: "",
            token = kv.getString("server.token_sealed")?.let(box::open) ?: "",
            connectTimeoutMs = kv.getLong("server.connect_timeout_ms", d.connectTimeoutMs),
            commitTimeoutBaseMs = kv.getLong("server.commit_timeout_base_ms", d.commitTimeoutBaseMs),
            healthTtlS = kv.getLong("server.health_ttl_s", d.healthTtlS),
            keepLocalWarm = kv.getBoolean("keep_local_warm", d.keepLocalWarm),
            showEngineBadge = kv.getBoolean("show_engine_badge", d.showEngineBadge),
        )
        // A toggle this build can't honour reads as off (e.g. prefs kept across a flavour switch).
        return stored.copy(allowServer = stored.allowServer && caps.remote, allowClient = stored.allowClient && caps.local)
    }

    fun save(s: DevSettings) = kv.edit {
        putBoolean("allow_server", s.allowServer)
        putBoolean("allow_client", s.allowClient)
        putString("route_order", s.routeOrder.joinToString(",") { it.wire })
        putString("server.host", s.host)
        putLong("server.port", s.port.toLong())
        putBoolean("server.tls", s.tls)
        putString("server.tls_pin_sha256", s.tlsPinSha256)
        putString("server.token_sealed", if (s.token.isEmpty()) null else box.seal(s.token))
        putLong("server.connect_timeout_ms", s.connectTimeoutMs)
        putLong("server.commit_timeout_base_ms", s.commitTimeoutBaseMs)
        putLong("server.health_ttl_s", s.healthTtlS)
        putBoolean("keep_local_warm", s.keepLocalWarm)
        putBoolean("show_engine_badge", s.showEngineBadge)
    }

    /** "Reset to defaults": everything except the unlock flag. */
    fun reset() {
        val unlocked = devUnlocked
        kv.edit { clear(); putBoolean("dev_unlocked", unlocked) }
    }

    var devUnlocked: Boolean
        get() = kv.getBoolean("dev_unlocked", false)
        set(v) = kv.edit { putBoolean("dev_unlocked", v) }

    val visible: Boolean get() = DevUnlock.visible(caps, devUnlocked)

    companion object {
        const val FILE = "dev"
    }
}
