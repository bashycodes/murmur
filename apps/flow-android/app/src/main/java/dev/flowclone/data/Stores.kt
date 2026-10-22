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

    fun load() = Settings(
        language = sp.getString("language", "auto") ?: "auto",
        cleanup = CleanupLevel.entries.firstOrNull { it.wire == sp.getString("cleanup", null) } ?: CleanupLevel.NONE,
        retention = Retention.fromKey(sp.getString("retention", null)),
        capMinutes = sp.getInt("cap_minutes", 5),
        mockEngine = sp.getBoolean("mock_engine", false),
        userDenylist = sp.getStringSet("user_denylist", emptySet())?.toSet() ?: emptySet(),
        llmEnabled = sp.getBoolean("llm_enabled", false),
        livePreview = sp.getBoolean("live_preview", true),
        shakeToReveal = sp.getBoolean("shake_to_reveal", true),
        shrinkBubble = sp.getBoolean("shrink_bubble", true),
        spokenCleanup = sp.getBoolean("spoken_cleanup", true),
        asrEngine = dev.flowclone.core.AsrEngine.entries.firstOrNull { it.wire == sp.getString("asr_engine", null) } ?: dev.flowclone.core.AsrEngine.WHISPER,
        endpointSilenceMs = sp.getInt("endpoint_silence_ms", Settings.DEFAULT_ENDPOINT_SILENCE_MS),
    )

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
            .putBoolean("spoken_cleanup", s.spokenCleanup)
            .putString("asr_engine", s.asrEngine.wire)
            .putInt("endpoint_silence_ms", s.endpointSilenceMs)
            .apply()
    }

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
)

/** Local-only history (plain SQLite; no audio stored). */
class HistoryStore(context: Context) : SQLiteOpenHelper(context, "history.db", null, 2) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE history(id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL,
               app_id TEXT, status TEXT NOT NULL, asr_text TEXT NOT NULL, final_text TEXT NOT NULL,
               inserted INTEGER NOT NULL, audio_ms INTEGER NOT NULL)""",
        )
        db.execSQL("CREATE INDEX history_created ON history(created_at)")
        addV2Columns(db)
    }

    /** v2: why the LLM fell back, and per-stage timings (the device latency checklist reads them). */
    private fun addV2Columns(db: SQLiteDatabase) {
        db.execSQL("ALTER TABLE history ADD COLUMN fallback_reason TEXT")
        for (c in listOf("asr_ms", "llm_ms", "total_ms")) db.execSQL("ALTER TABLE history ADD COLUMN $c INTEGER NOT NULL DEFAULT 0")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) addV2Columns(db)
    }

    fun add(row: HistoryRow, retention: Retention) {
        if (!retention.stores) return
        writableDatabase.insert("history", null, ContentValues().apply {
            put("created_at", row.createdAt); put("app_id", row.appId); put("status", row.status)
            put("asr_text", row.asrText); put("final_text", row.finalText)
            put("inserted", if (row.inserted) 1 else 0); put("audio_ms", row.audioMs)
            put("fallback_reason", row.fallbackReason); put("asr_ms", row.asrMs); put("llm_ms", row.llmMs); put("total_ms", row.totalMs)
        })
        purge(retention, row.createdAt)
    }

    fun purge(retention: Retention, now: Long = System.currentTimeMillis()) {
        val cutoff = retention.cutoff(now) ?: return
        writableDatabase.delete("history", "created_at < ?", arrayOf(cutoff.toString()))
    }

    fun delete(id: Long) { writableDatabase.delete("history", "id = ?", arrayOf(id.toString())) }

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
                    ),
                )
            }
        }

    fun clear() { writableDatabase.delete("history", null, null) }
}
