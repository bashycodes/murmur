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
    )

    fun save(s: Settings) {
        sp.edit()
            .putString("language", s.language)
            .putString("cleanup", s.cleanup.wire)
            .putString("retention", s.retention.key)
            .putInt("cap_minutes", s.capMinutes)
            .putBoolean("mock_engine", s.mockEngine)
            .putStringSet("user_denylist", s.userDenylist)
            .apply()
    }

    var modelPath: String?
        get() = sp.getString("model_path", null)
        set(v) { sp.edit().putString("model_path", v).apply() }

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
)

/** Local-only history (plain SQLite; no audio stored). */
class HistoryStore(context: Context) : SQLiteOpenHelper(context, "history.db", null, 1) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE history(id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL,
               app_id TEXT, status TEXT NOT NULL, asr_text TEXT NOT NULL, final_text TEXT NOT NULL,
               inserted INTEGER NOT NULL, audio_ms INTEGER NOT NULL)""",
        )
        db.execSQL("CREATE INDEX history_created ON history(created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit

    fun add(row: HistoryRow, retention: Retention) {
        if (!retention.stores) return
        writableDatabase.insert("history", null, ContentValues().apply {
            put("created_at", row.createdAt); put("app_id", row.appId); put("status", row.status)
            put("asr_text", row.asrText); put("final_text", row.finalText)
            put("inserted", if (row.inserted) 1 else 0); put("audio_ms", row.audioMs)
        })
        purge(retention, row.createdAt)
    }

    fun purge(retention: Retention, now: Long = System.currentTimeMillis()) {
        val cutoff = retention.cutoff(now) ?: return
        writableDatabase.delete("history", "created_at < ?", arrayOf(cutoff.toString()))
    }

    fun recent(limit: Int = 200): List<HistoryRow> =
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
                    ),
                )
            }
        }

    fun clear() { writableDatabase.delete("history", null, null) }
}
