package dev.flowclone.data

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.util.Base64
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

/** Encrypts secrets at rest (the server token, and per provider the cloud STT API keys). */
interface SecretBox {
    fun seal(plain: String): String
    /** Null when the ciphertext can't be opened (key lost after a restore, tampering). */
    fun open(sealed: String): String?
    /** Forget the wrapping key (provider deleted); default: nothing to forget. */
    fun destroy() = Unit
}

/** Where a [SecretVault]'s AES key lives: the Android Keystore on a device, a JVM fake in tests. */
interface VaultKeys {
    /** The AES-256 key for [alias], created on first use. */
    fun key(alias: String): SecretKey
    fun delete(alias: String)
}

/** Non-exportable Android Keystore AES-GCM keys (§2.5 / D18 §8.1). */
object AndroidKeystoreKeys : VaultKeys {
    private fun store() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    override fun key(alias: String): SecretKey {
        (store().getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(
                KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
        }.generateKey()
    }

    override fun delete(alias: String) {
        runCatching { store().deleteEntry(alias) }
    }
}

/**
 * AES-GCM under the Keystore key [alias]; only ciphertext ("iv:ct", base64) is stored.
 * `flow_dev_token` seals the server token (D17 §2.5); `flow_cloud_<id>` one cloud STT API key
 * per provider (D18 §8.1), so deleting a provider can destroy its key alone.
 */
class SecretVault(val alias: String, private val keys: VaultKeys = AndroidKeystoreKeys) : SecretBox {
    override fun seal(plain: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, keys.key(alias)) }
        val ct = c.doFinal(plain.toByteArray(Charsets.UTF_8))
        val b = Base64.getEncoder()
        return b.encodeToString(c.iv) + ":" + b.encodeToString(ct)
    }

    override fun open(sealed: String): String? = runCatching {
        val (iv, ct) = sealed.split(':', limit = 2).map { Base64.getDecoder().decode(it) }
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, keys.key(alias), GCMParameterSpec(128, iv)) }
        String(c.doFinal(ct), Charsets.UTF_8)
    }.getOrNull()

    override fun destroy() = keys.delete(alias)

    override fun toString() = "SecretVault($alias)"

    companion object {
        fun cloudAlias(providerId: String) = "flow_cloud_$providerId"
    }
}

/** §2.5: the server token's vault (same format as before the generalisation). */
val TokenVault: SecretBox = SecretVault("flow_dev_token")

/**
 * D18 C3: cloud STT API keys at rest. One [SecretVault] (`flow_cloud_<id>`) per provider; only the
 * ciphertext is in the `dev` prefs (`cloud.key_sealed.<id>`, backup disabled app-wide). Keys never
 * enter [DevSettings], router JSON, logs or exports; they reach native code only through
 * `NativeFlowCore.setCloudKey` / `probeCloud`.
 */
class CloudKeyStore(private val kv: KeyValue, private val vaultFor: (String) -> SecretBox) {
    constructor(context: Context) : this(
        PrefsKeyValue(context.getSharedPreferences(DevStore.FILE, Context.MODE_PRIVATE)),
        { id -> SecretVault(SecretVault.cloudAlias(id)) },
    )

    private fun k(id: String) = "$PREFIX$id"

    fun get(id: String): String? = kv.getString(k(id))?.let { vaultFor(id).open(it) }?.takeIf { it.isNotEmpty() }

    fun has(id: String) = kv.contains(k(id))

    /** Blank clears. */
    fun set(id: String, key: String?) {
        val v = key?.trim().orEmpty()
        if (v.isEmpty()) { remove(id); return }
        val sealed = vaultFor(id).seal(v)
        kv.edit { putString(k(id), sealed) }
    }

    /** Provider deleted: drop the ciphertext and its Keystore key. */
    fun remove(id: String) {
        kv.edit { remove(k(id)) }
        runCatching { vaultFor(id).destroy() }
    }

    /** UI: `••••abcd` or "no key". */
    fun masked(id: String) = CloudText.mask(get(id))

    companion object {
        const val PREFIX = "cloud.key_sealed."
    }
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
            connectTimeoutMs = loadConnectTimeout(),
            commitTimeoutBaseMs = kv.getLong("server.commit_timeout_base_ms", d.commitTimeoutBaseMs),
            healthTtlS = kv.getLong("server.health_ttl_s", d.healthTtlS),
            keepLocalWarm = kv.getBoolean("keep_local_warm", d.keepLocalWarm),
            showEngineBadge = kv.getBoolean("show_engine_badge", d.showEngineBadge),
            allowCloud = kv.getBoolean("allow_cloud", false),
            cloud = kv.getString("cloud")?.let { runCatching { CloudSettings.fromJson(org.json.JSONObject(it)) }.getOrNull() } ?: CloudSettings(),
        )
        // A toggle this build can't honour reads as off (e.g. prefs kept across a flavour switch).
        return stored.copy(
            allowServer = stored.allowServer && caps.remote, allowClient = stored.allowClient && caps.local,
            allowCloud = stored.allowCloud && caps.cloud,
        )
    }

    /** D24: an untouched stored 800 ms (the pre-D24 default) becomes 2500 ms, once. */
    private fun loadConnectTimeout(): Long {
        val stored = if (kv.contains(CONNECT_TIMEOUT)) kv.getLong(CONNECT_TIMEOUT, DevSettings.DEFAULT_CONNECT_TIMEOUT_MS) else null
        val migrated = kv.getBoolean(CONNECT_TIMEOUT_MIGRATED, false)
        val v = DevSettings.migrateConnectTimeout(stored, migrated)
        if (!migrated) kv.edit {
            putBoolean(CONNECT_TIMEOUT_MIGRATED, true)
            if (stored != null && v != stored) putLong(CONNECT_TIMEOUT, v)
        }
        return v
    }

    fun save(s: DevSettings) = kv.edit {
        putBoolean(CONNECT_TIMEOUT_MIGRATED, true)
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
        putBoolean("allow_cloud", s.allowCloud)
        putString("cloud", if (s.cloud.isDefault) null else s.cloud.toJson().toString())
    }

    /** Only the cloud part (provider page saves, consent dialog), leaving unsaved form fields alone. */
    fun saveCloud(allowCloud: Boolean? = null, cloud: CloudSettings, routeOrder: List<DevSettings.Route>? = null) = kv.edit {
        allowCloud?.let { putBoolean("allow_cloud", it) }
        putString("cloud", if (cloud.isDefault) null else cloud.toJson().toString())
        routeOrder?.let { o -> putString("route_order", o.joinToString(",") { it.wire }) }
    }

    /**
     * "Reset to defaults": everything except the unlock flag and this month's cloud usage
     * (sealed cloud keys go too; their Keystore keys are destroyed by [CloudKeyStore.remove]).
     */
    fun reset() {
        val unlocked = devUnlocked
        val usage = cloudUsageJson
        kv.edit { clear(); putBoolean("dev_unlocked", unlocked); putString(USAGE, usage) }
    }

    /** D18: this month's `CloudUsage` JSON (router counters, persisted after each cloud commit). */
    var cloudUsageJson: String?
        get() = kv.getString(USAGE)
        set(v) = kv.edit { putString(USAGE, v) }

    var devUnlocked: Boolean
        get() = kv.getBoolean("dev_unlocked", false)
        set(v) = kv.edit { putBoolean("dev_unlocked", v) }

    val visible: Boolean get() = DevUnlock.visible(caps, devUnlocked)

    /** D21: "Debug log (includes transcripts)", default off; every flavour (it never leaves the device). */
    var debugLog: Boolean
        get() = kv.getBoolean(DEBUG_LOG, false)
        set(v) = kv.edit { putBoolean(DEBUG_LOG, v) }

    /**
     * "Keep last dictation audio (debug)", default off: the last few dictations' 16 kHz PCM as WAV
     * in app-private `files/debug/audio/` (never for protected targets), deleted after 24 h.
     */
    var keepDebugAudio: Boolean
        get() = kv.getBoolean(KEEP_DEBUG_AUDIO, false)
        set(v) = kv.edit { putBoolean(KEEP_DEBUG_AUDIO, v) }

    /** The ASR model the server reported in its last successful health probe (debug log `model`). */
    var serverAsrModel: String?
        get() = kv.getString(SERVER_ASR_MODEL)
        set(v) = kv.edit { putString(SERVER_ASR_MODEL, v) }

    /**
     * D22: Parakeet search mode for the on-device engine. Default greedy: one recognizer, ≈ 0.6 GB
     * less RAM than `auto` (which adds a beam recognizer for dictionary hotwords).
     */
    var parakeetDecoding: dev.flowclone.core.ParakeetDecoding
        get() = dev.flowclone.core.ParakeetDecoding.fromWire(kv.getString(PARAKEET_DECODING)) ?: dev.flowclone.core.ParakeetDecoding.GREEDY
        set(v) = kv.edit { putString(PARAKEET_DECODING, v.wire) }

    companion object {
        const val PARAKEET_DECODING = "parakeet_decoding"
        const val FILE = "dev"
        private const val CONNECT_TIMEOUT = "server.connect_timeout_ms"
        /** D24: set once the 800 → 2500 ms connect-timeout migration ran (or on any save). */
        private const val CONNECT_TIMEOUT_MIGRATED = "connect_timeout_default_2500"
        const val USAGE = "cloud_usage"
        const val DEBUG_LOG = "debug_log"
        const val KEEP_DEBUG_AUDIO = "debug_keep_audio"
        const val SERVER_ASR_MODEL = "server.last_asr_model"
    }
}
