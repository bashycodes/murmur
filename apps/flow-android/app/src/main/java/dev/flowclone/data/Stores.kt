package dev.flowclone.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import dev.flowclone.bubble.BubblePositioner
import dev.flowclone.core.CleanupLevel
import dev.flowclone.core.CoreJson
import dev.flowclone.core.DictEntry
import java.io.File

/** SharedPreferences-backed [Settings] + model path + bubble position. */
class SettingsStore(context: Context) {
    private val sp = context.getSharedPreferences("flow_settings", Context.MODE_PRIVATE)

    private companion object {
        /** D22: the hands-free endpoint default moved 2000 → 1500 ms; set once that ran (or on any save). */
        const val ENDPOINT_MIGRATED = "endpoint_default_1500"
        /** 2026-10-23: the language default moved auto → phone language; set once that ran (or on any save). */
        const val LANGUAGE_MIGRATED = "language_default_device"
    }

    fun load() = Settings(
        language = loadLanguage(),
        cleanup = CleanupLevel.entries.firstOrNull { it.wire == sp.getString("cleanup", null) } ?: CleanupLevel.NONE,
        retention = Retention.fromKey(sp.getString("retention", null)),
        capMinutes = sp.getInt("cap_minutes", 5),
        mockEngine = sp.getBoolean("mock_engine", false),
        userDenylist = sp.getStringSet("user_denylist", emptySet())?.toSet() ?: emptySet(),
        llmEnabled = sp.getBoolean("llm_enabled", false),
        livePreview = sp.getBoolean("live_preview", true),
        shakeToReveal = sp.getBoolean("shake_to_reveal", true),
        shrinkBubble = sp.getBoolean("shrink_bubble", true),
        shrinkToDot = loadShrinkToDot(),
        bubbleScale = loadBubbleScale(),
        spokenCleanup = sp.getBoolean("spoken_cleanup", true),
        asrEngine = dev.flowclone.core.AsrEngine.entries.firstOrNull { it.wire == sp.getString("asr_engine", null) } ?: dev.flowclone.core.AsrEngine.WHISPER,
        endpointSilenceMs = loadEndpointMs(),
        decodeWhileSpeaking = sp.getBoolean("decode_while_speaking", true),
        styles = loadStyles(),
        appBuckets = WritingStyle.parseOverrides(sp.getString("app_buckets", null).orEmpty()).map,
        learnFromEdits = sp.getBoolean("learn_from_edits", true),
    )

    /** D27: nothing stored (every install before 0.3) = the [dev.flowclone.core.Styles] defaults. */
    private fun loadStyles(): dev.flowclone.core.Styles {
        val d = dev.flowclone.core.Styles()
        fun st(k: String, def: dev.flowclone.core.Style) = dev.flowclone.core.Style.fromWire(sp.getString(k, null)) ?: def
        return dev.flowclone.core.Styles(
            personal = st("style_personal", d.personal), work = st("style_work", d.work),
            email = st("style_email", d.email), other = st("style_other", d.other),
        )
    }

    /** One-time move of an untouched stored "auto" to unset ([Settings.migrateLanguage]). */
    private fun loadLanguage(): String {
        val stored = sp.getString("language", null)
        val migrated = sp.getBoolean(LANGUAGE_MIGRATED, false)
        val v = Settings.migrateLanguage(stored, migrated)
        if (!migrated) sp.edit().putBoolean(LANGUAGE_MIGRATED, true).putString("language", v).apply()
        return v
    }

    /** D22: one-time move of an untouched 2000 ms endpoint to the new 1500 default ([Settings.migrateEndpointMs]). */
    private fun loadEndpointMs(): Int {
        val stored = if (sp.contains("endpoint_silence_ms")) sp.getInt("endpoint_silence_ms", Settings.DEFAULT_ENDPOINT_SILENCE_MS) else null
        val migrated = sp.getBoolean(ENDPOINT_MIGRATED, false)
        val v = Settings.migrateEndpointMs(stored, migrated)
        if (!migrated) {
            val e = sp.edit().putBoolean(ENDPOINT_MIGRATED, true)
            if (stored != null && v != stored) e.putInt("endpoint_silence_ms", v)
            e.apply()
        }
        return v
    }

    fun save(s: Settings) {
        sp.edit()
            .putString("language", s.language)
            .putString("cleanup", s.cleanup.wire)
            .putString("retention", s.retention.key)
            .putInt("cap_minutes", s.capMinutes)
            .putBoolean("mock_engine", s.mockEngine)
            .putStringSet("user_denylist", s.userDenylist)
            .putBoolean("llm_enabled", s.llmEnabled)
            .putBoolean("live_preview", s.livePreview)
            .putBoolean("shake_to_reveal", s.shakeToReveal)
            .putBoolean("shrink_bubble", s.shrinkBubble)
            .putBoolean("shrink_to_dot", s.shrinkToDot)
            .putFloat("bubble_scale", s.bubbleScale)
            .putBoolean("spoken_cleanup", s.spokenCleanup)
            .putString("asr_engine", s.asrEngine.wire)
            .putInt("endpoint_silence_ms", s.endpointSilenceMs)
            .putBoolean(ENDPOINT_MIGRATED, true)
            .putBoolean(LANGUAGE_MIGRATED, true)
            .putBoolean("decode_while_speaking", s.decodeWhileSpeaking)
            .putString("style_personal", s.styles.personal.wire)
            .putString("style_work", s.styles.work.wire)
            .putString("style_email", s.styles.email.wire)
            .putString("style_other", s.styles.other.wire)
            .putString("app_buckets", WritingStyle.formatOverrides(s.appBuckets))
            .putBoolean("learn_from_edits", s.learnFromEdits)
            .apply()
        _bubbleScale.value = s.bubbleScale
        _shrinkToDot.value = s.shrinkToDot
    }

    private fun loadBubbleScale() = Settings.bubbleScaleOf(sp.getFloat("bubble_scale", 1f))
    private val _bubbleScale = kotlinx.coroutines.flow.MutableStateFlow(loadBubbleScale())
    /** The saved "Bubble size", so the running bubble resizes the moment it changes. */
    val bubbleScale: kotlinx.coroutines.flow.StateFlow<Float> = _bubbleScale

    private fun loadShrinkToDot() = sp.getBoolean("shrink_to_dot", true)
    private val _shrinkToDot = kotlinx.coroutines.flow.MutableStateFlow(loadShrinkToDot())
    /** The saved "Shrink to a dot", so a minimised bubble redraws the moment it changes. */
    val shrinkToDot: kotlinx.coroutines.flow.StateFlow<Boolean> = _shrinkToDot

    var modelPath: String?
        get() = sp.getString("model_path", null)
        set(v) { sp.edit().putString("model_path", v).apply() }

    /** Imported Parakeet model directory (filesDir/parakeet/<name>/). */
    var parakeetPath: String?
        get() = sp.getString("parakeet_path", null)
        set(v) { sp.edit().putString("parakeet_path", v).apply() }

    /** Imported GGUF for LLM cleanup (filesDir/llm/…). */
    var llmModelPath: String?
        get() = sp.getString("llm_model_path", null)
        set(v) { sp.edit().putString("llm_model_path", v).apply() }

    /** The user pressed "Stop" on the service; the watchdog and boot receiver leave it stopped. */
    var serviceStoppedByUser: Boolean
        get() = sp.getBoolean("service_stopped_by_user", false)
        set(v) { sp.edit().putBoolean("service_stopped_by_user", v).apply() }

    val bubblePosition = object : BubblePositioner.Store {
    override fun load(): BubblePositioner.Saved? {
        val side = sp.getString("bubble_side", null) ?: return null
        return BubblePositioner.Saved(
            runCatching { BubblePositioner.Side.valueOf(side) }.getOrDefault(BubblePositioner.Side.RIGHT),
            sp.getInt("bubble_y", BubblePositioner.DEFAULT_Y_INDEX),
        )
    }

    override fun save(s: BubblePositioner.Saved) {
        sp.edit().putString("bubble_side", s.side.name).putInt("bubble_y", s.yIndex).apply()
    }
    }
}

/** Dictionary + snippets persisted as the shared export JSON `[{word, replacement, isSnippet}]`. */
class DictionaryStore(context: Context) {
    private val file = File(context.filesDir, "dictionary.json")
    @Volatile private var cache: List<DictEntry>? = null

    @Synchronized
    fun all(): List<DictEntry> = cache ?: (if (file.exists()) runCatching { CoreJson.decodeDictionary(file.readText()) }.getOrDefault(emptyList()) else emptyList())
        .also { cache = it }

    @Synchronized
    fun replaceAll(entries: List<DictEntry>) {
        val tmp = File(file.parentFile, "dictionary.json.tmp")
        tmp.writeText(CoreJson.encodeDictionary(entries))
        tmp.renameTo(file)
        cache = entries
    }

    fun upsert(e: DictEntry) = replaceAll(DictionaryMerge.merge(all(), listOf(e)))
    fun remove(word: String) = replaceAll(all().filterNot { it.word.equals(word, ignoreCase = true) })
    fun importJson(json: String): Int { val inc = CoreJson.decodeDictionary(json); replaceAll(DictionaryMerge.merge(all(), inc)); return inc.size }
    fun exportJson(): String = CoreJson.encodeDictionary(all())
}

/** Learn-from-edits suggestions ([dev.flowclone.learn.Suggestions]): words only, `learn_suggestions.json`. */
class SuggestionStore(context: Context) {
    private val file = File(context.filesDir, "learn_suggestions.json")
    @Volatile private var cache: dev.flowclone.learn.Suggestions? = null

    @Synchronized
    fun load(): dev.flowclone.learn.Suggestions = cache
        ?: dev.flowclone.learn.Suggestions.decode(runCatching { file.takeIf { it.exists() }?.readText() }.getOrNull()).also { cache = it }

    /** Read-modify-write under the store lock; [f] returns the new value (same = no write). */
    @Synchronized
    fun update(f: (dev.flowclone.learn.Suggestions) -> dev.flowclone.learn.Suggestions): dev.flowclone.learn.Suggestions {
        val old = load()
        val new = f(old)
        if (new != old) {
            val tmp = File(file.parentFile, "learn_suggestions.json.tmp")
            tmp.writeText(dev.flowclone.learn.Suggestions.encode(new))
            tmp.renameTo(file)
            cache = new
        }
        return new
    }
}

data class HistoryRow(
    val id: Long,
    val createdAt: Long,
    val appId: String?,
    val status: String,
    val asrText: String,
    val finalText: String,
    val inserted: Boolean,
    val audioMs: Long,
    val fallbackReason: String? = null,
    val asrMs: Long = 0,
    val llmMs: Long = 0,
    val totalMs: Long = 0,
    /**
     * v3: how the insert ended ([dev.flowclone.debug.InsertOutcome] wire values: inserted,
     * target_changed, target_protected, paste_failed, empty, error); null = saved before v3.
     */
    val insertOutcome: String? = null,
    /** v3: [dev.flowclone.core.RouteInfo.engine] (local / remote / cloud); null = unknown. */
    val route: String? = null,
)

/** Local-only history (plain SQLite; no audio stored). */
/**
 * Review S17: deleted transcripts must not stay readable in free pages or the WAL file.
 * `secure_delete` zeroes freed content on every connection; after a delete / purge / clear the
 * WAL is checkpointed and truncated, and a full clear also VACUUMs the file.
 */
object HistoryHygiene {
    const val ON_OPEN = "PRAGMA secure_delete=ON"
    const val AFTER_DELETE = "PRAGMA wal_checkpoint(TRUNCATE)"
    const val AFTER_CLEAR = "VACUUM"
}

class HistoryStore(context: Context) : SQLiteOpenHelper(context, "history.db", null, 3) {
    override fun onConfigure(db: SQLiteDatabase) {
        super.onConfigure(db)
        db.rawQuery(HistoryHygiene.ON_OPEN, null).use { it.moveToFirst() }
    }

    /** S17: flush deletions out of the WAL (no-op without WAL). */
    private fun scrub(vacuum: Boolean = false) {
        runCatching {
            val db = writableDatabase
            db.rawQuery(HistoryHygiene.AFTER_DELETE, null).use { it.moveToFirst() }
            if (vacuum) db.execSQL(HistoryHygiene.AFTER_CLEAR)
        }
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE history(id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL,
               app_id TEXT, status TEXT NOT NULL, asr_text TEXT NOT NULL, final_text TEXT NOT NULL,
               inserted INTEGER NOT NULL, audio_ms INTEGER NOT NULL)""",
        )
        db.execSQL("CREATE INDEX history_created ON history(created_at)")
        addV2Columns(db)
        addV3Columns(db)
    }

    /** v3: the insert outcome and route (fix/route-order); NULL for older rows = unknown. */
    private fun addV3Columns(db: SQLiteDatabase) {
        for (c in listOf("insert_outcome", "route")) db.execSQL("ALTER TABLE history ADD COLUMN $c TEXT")
    }

    /** v2: why the LLM fell back, and per-stage timings (the device latency checklist reads them). */
    private fun addV2Columns(db: SQLiteDatabase) {
        db.execSQL("ALTER TABLE history ADD COLUMN fallback_reason TEXT")
        for (c in listOf("asr_ms", "llm_ms", "total_ms")) db.execSQL("ALTER TABLE history ADD COLUMN $c INTEGER NOT NULL DEFAULT 0")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) addV2Columns(db)
        if (oldVersion < 3) addV3Columns(db)
    }

    fun add(row: HistoryRow, retention: Retention) {
        if (!retention.stores) return
        writableDatabase.insert("history", null, ContentValues().apply {
            put("created_at", row.createdAt); put("app_id", row.appId); put("status", row.status)
            put("asr_text", row.asrText); put("final_text", row.finalText)
            put("inserted", if (row.inserted) 1 else 0); put("audio_ms", row.audioMs)
            put("fallback_reason", row.fallbackReason); put("asr_ms", row.asrMs); put("llm_ms", row.llmMs); put("total_ms", row.totalMs)
            put("insert_outcome", row.insertOutcome); put("route", row.route)
        })
        purge(retention, row.createdAt)
    }

    fun purge(retention: Retention, now: Long = System.currentTimeMillis()) {
        val cutoff = retention.cutoff(now) ?: return
        if (writableDatabase.delete("history", "created_at < ?", arrayOf(cutoff.toString())) > 0) scrub(vacuum = retention == Retention.NEVER)
    }

    fun delete(id: Long) { writableDatabase.delete("history", "id = ?", arrayOf(id.toString())); scrub() }

    fun recent(limit: Int = 500): List<HistoryRow> =
        readableDatabase.query("history", null, null, null, null, null, "created_at DESC", limit.toString()).use { c ->
            buildList {
                while (c.moveToNext()) add(
                    HistoryRow(
                        id = c.getLong(c.getColumnIndexOrThrow("id")),
                        createdAt = c.getLong(c.getColumnIndexOrThrow("created_at")),
                        appId = c.getString(c.getColumnIndexOrThrow("app_id")),
                        status = c.getString(c.getColumnIndexOrThrow("status")),
                        asrText = c.getString(c.getColumnIndexOrThrow("asr_text")),
                        finalText = c.getString(c.getColumnIndexOrThrow("final_text")),
                        inserted = c.getInt(c.getColumnIndexOrThrow("inserted")) != 0,
                        audioMs = c.getLong(c.getColumnIndexOrThrow("audio_ms")),
                        fallbackReason = c.getString(c.getColumnIndexOrThrow("fallback_reason")),
                        asrMs = c.getLong(c.getColumnIndexOrThrow("asr_ms")),
                        llmMs = c.getLong(c.getColumnIndexOrThrow("llm_ms")),
                        totalMs = c.getLong(c.getColumnIndexOrThrow("total_ms")),
                        insertOutcome = c.getString(c.getColumnIndexOrThrow("insert_outcome")),
                        route = c.getString(c.getColumnIndexOrThrow("route")),
                    ),
                )
            }
        }

    fun clear() { writableDatabase.delete("history", null, null); scrub(vacuum = true) }
}
