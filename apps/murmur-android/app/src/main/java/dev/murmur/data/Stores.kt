package dev.murmur.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import dev.murmur.bubble.BubblePositioner
import dev.murmur.core.CleanupLevel
import dev.murmur.core.CoreJson
import dev.murmur.core.DictEntry
import java.io.File

/** SharedPreferences-backed [Settings] + model path + bubble position. */
class SettingsStore(context: Context) {
    private val sp = context.getSharedPreferences("murmur_settings", Context.MODE_PRIVATE)

    private companion object {
        /** D22: the hands-free endpoint default moved 2000 → 1500 ms; set once that ran (or on any save). */
        const val ENDPOINT_MIGRATED = "endpoint_default_1500"
        /** 2026-10-23: the language default moved auto → phone language; set once that ran (or on any save). */
        const val LANGUAGE_MIGRATED = "language_default_device"
        /** D37: the default moved phone language → keyboard language; set once that ran (or on any save). */
        const val KEYBOARD_MIGRATED = "language_default_keyboard"
    }

    fun load() = Settings(
        language = loadLanguage(),
        cleanup = CleanupLevel.entries.firstOrNull { it.wire == sp.getString("cleanup", null) } ?: CleanupLevel.NONE,
        retention = Retention.fromKey(sp.getString("retention", null)),
        retryAudioKeep = RetryLogic.keepOf(sp.getInt("retry_audio_keep", RetryLogic.DEFAULT_KEEP)),
        capMinutes = sp.getInt("cap_minutes", 5),
        mockEngine = sp.getBoolean("mock_engine", false),
        userDenylist = sp.getStringSet("user_denylist", emptySet())?.toSet() ?: emptySet(),
        llmEnabled = sp.getBoolean("llm_enabled", false),
        livePreview = sp.getBoolean("live_preview", Settings.DEFAULT_LIVE_PREVIEW),
        shakeToReveal = sp.getBoolean("shake_to_reveal", true),
        shrinkBubble = sp.getBoolean("shrink_bubble", true),
        shrinkToDot = loadShrinkToDot(),
        bubbleScale = loadBubbleScale(),
        commandButton = loadCommandButton(),
        spokenCleanup = sp.getBoolean("spoken_cleanup", true),
        asrEngine = dev.murmur.core.AsrEngine.entries.firstOrNull { it.wire == sp.getString("asr_engine", null) } ?: dev.murmur.core.AsrEngine.WHISPER,
        endpointSilenceMs = loadEndpointMs(),
        decodeWhileSpeaking = sp.getBoolean("decode_while_speaking", true),
        styles = loadStyles(),
        appBuckets = WritingStyle.parseOverrides(sp.getString("app_buckets", null).orEmpty()).map,
        learnFromEdits = sp.getBoolean("learn_from_edits", true),
        keepBothLanguages = sp.getBoolean("keep_both_languages", false),
    )

    /** D27: nothing stored (every install before 0.3) = the [dev.murmur.core.Styles] defaults. */
    private fun loadStyles(): dev.murmur.core.Styles {
        val d = dev.murmur.core.Styles()
        fun st(k: String, def: dev.murmur.core.Style) = dev.murmur.core.Style.fromWire(sp.getString(k, null)) ?: def
        return dev.murmur.core.Styles(
            personal = st("style_personal", d.personal), work = st("style_work", d.work),
            email = st("style_email", d.email), other = st("style_other", d.other),
        )
    }

    /**
     * One-time moves: an untouched stored "auto" to phone language ([Settings.migrateLanguage]),
     * then phone language to keyboard language (D37, [Settings.migrateKeyboardLanguage]).
     */
    private fun loadLanguage(): String {
        val stored = sp.getString("language", null)
        val migrated = sp.getBoolean(LANGUAGE_MIGRATED, false)
        val keyboard = sp.getBoolean(KEYBOARD_MIGRATED, false)
        val v = Settings.migrateKeyboardLanguage(stored, migrated, keyboard)
        if (!migrated || !keyboard) {
            sp.edit().putBoolean(LANGUAGE_MIGRATED, true).putBoolean(KEYBOARD_MIGRATED, true).putString("language", v).apply()
        }
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
            .putInt("retry_audio_keep", RetryLogic.keepOf(s.retryAudioKeep))
            .putInt("cap_minutes", s.capMinutes)
            .putBoolean("mock_engine", s.mockEngine)
            .putStringSet("user_denylist", s.userDenylist)
            .putBoolean("llm_enabled", s.llmEnabled)
            .putBoolean("live_preview", s.livePreview)
            .putBoolean("shake_to_reveal", s.shakeToReveal)
            .putBoolean("shrink_bubble", s.shrinkBubble)
            .putBoolean("shrink_to_dot", s.shrinkToDot)
            .putFloat("bubble_scale", s.bubbleScale)
            .putString("command_button", s.commandButton.key)
            .putBoolean("spoken_cleanup", s.spokenCleanup)
            .putString("asr_engine", s.asrEngine.wire)
            .putInt("endpoint_silence_ms", s.endpointSilenceMs)
            .putBoolean(ENDPOINT_MIGRATED, true)
            .putBoolean(LANGUAGE_MIGRATED, true)
            .putBoolean(KEYBOARD_MIGRATED, true)
            .putBoolean("decode_while_speaking", s.decodeWhileSpeaking)
            .putString("style_personal", s.styles.personal.wire)
            .putString("style_work", s.styles.work.wire)
            .putString("style_email", s.styles.email.wire)
            .putString("style_other", s.styles.other.wire)
            .putString("app_buckets", WritingStyle.formatOverrides(s.appBuckets))
            .putBoolean("learn_from_edits", s.learnFromEdits)
            .putBoolean("keep_both_languages", s.keepBothLanguages)
            .apply()
        _bubbleScale.value = s.bubbleScale
        _shrinkToDot.value = s.shrinkToDot
        _commandButton.value = s.commandButton
    }

    private fun loadCommandButton() = CommandButton.fromKey(sp.getString("command_button", null))
    private val _commandButton = kotlinx.coroutines.flow.MutableStateFlow(loadCommandButton())
    /** The saved "Command button", so the running bubble shows / hides ✎ the moment it changes. */
    val commandButton: kotlinx.coroutines.flow.StateFlow<CommandButton> = _commandButton

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

    /** H5: the Hebrew model slot, an imported directory (filesDir/hebrew/<kind>/), see [HebrewModels]. */
    var hebrewModelPath: String?
        get() = sp.getString("hebrew_model_path", null)
        set(v) { sp.edit().putString("hebrew_model_path", v).apply() }

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

    /**
     * D68: runs after every save, outside the store lock (the engine reloads when the dictionary
     * got its first word or lost its last, and its load reads the dictionary).
     */
    @Volatile var onChange: (() -> Unit)? = null

    fun replaceAll(entries: List<DictEntry>) {
        write(entries)
        onChange?.invoke()
    }

    @Synchronized
    private fun write(entries: List<DictEntry>) {
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

/** Learn-from-edits suggestions ([dev.murmur.learn.Suggestions]): words only, `learn_suggestions.json`. */
class SuggestionStore(context: Context) {
    private val file = File(context.filesDir, "learn_suggestions.json")
    @Volatile private var cache: dev.murmur.learn.Suggestions? = null

    @Synchronized
    fun load(): dev.murmur.learn.Suggestions = cache
        ?: dev.murmur.learn.Suggestions.decode(runCatching { file.takeIf { it.exists() }?.readText() }.getOrNull()).also { cache = it }

    /** Read-modify-write under the store lock; [f] returns the new value (same = no write). */
    @Synchronized
    fun update(f: (dev.murmur.learn.Suggestions) -> dev.murmur.learn.Suggestions): dev.murmur.learn.Suggestions {
        val old = load()
        val new = f(old)
        if (new != old) {
            val tmp = File(file.parentFile, "learn_suggestions.json.tmp")
            tmp.writeText(dev.murmur.learn.Suggestions.encode(new))
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
     * v3: how the insert ended ([dev.murmur.debug.InsertOutcome] wire values: inserted,
     * target_changed, target_protected, paste_failed, empty, error); null = saved before v3.
     */
    val insertOutcome: String? = null,
    /** v3: [dev.murmur.core.RouteInfo.engine] (local / remote / cloud); null = unknown. */
    val route: String? = null,
    /**
     * v4, Command Mode rows only ([status] = [COMMAND]): the selected text the instruction
     * rewrote; null = speak-to-generate. [asrText] holds the instruction, [finalText] the result.
     */
    val commandOriginal: String? = null,
    /**
     * v5: the writing-style bucket ([FieldContext.appCategory][dev.murmur.core.FieldContext.appCategory])
     * of the dictation's app, so a retry picks the same style; null = saved before v5.
     */
    val appCategory: String? = null,
) {
    val isCommand: Boolean get() = status == COMMAND

    companion object {
        /** [status] of a Command Mode row (not a [dev.murmur.core.CommitStatus]). */
        const val COMMAND = "COMMAND"

        fun ofCommand(r: dev.murmur.command.CommandRecord, createdAt: Long) = HistoryRow(
            0, createdAt, r.appId, COMMAND, r.instruction, r.result.orEmpty(),
            inserted = r.outcome == dev.murmur.debug.InsertOutcome.INSERTED, audioMs = 0,
            insertOutcome = r.outcome, commandOriginal = r.original,
        )
    }
}

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

/**
 * Local-only history (plain SQLite). D35: the recordings of the newest rows live beside it in
 * [audio] (for "Retry with current settings"); a recording and a row's versions go with the row.
 */
class HistoryStore(
    context: Context,
    val audio: RetryAudioStore = RetryAudioStore(File(context.filesDir, "retry_audio")),
) : SQLiteOpenHelper(context, "history.db", null, 5) {
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
        addV4Columns(db)
        addV5(db)
    }

    /** v5 (D35): the app's style bucket, and the earlier texts of retried rows (0 = the original). */
    private fun addV5(db: SQLiteDatabase) {
        db.execSQL("ALTER TABLE history ADD COLUMN app_category TEXT")
        db.execSQL(
            """CREATE TABLE history_versions(id INTEGER PRIMARY KEY AUTOINCREMENT, history_id INTEGER NOT NULL,
               n INTEGER NOT NULL, created_at INTEGER NOT NULL, status TEXT NOT NULL, asr_text TEXT NOT NULL,
               final_text TEXT NOT NULL, route TEXT, UNIQUE(history_id, n))""",
        )
    }

    /** v4: Command Mode's original selection (status COMMAND rows only). */
    private fun addV4Columns(db: SQLiteDatabase) {
        db.execSQL("ALTER TABLE history ADD COLUMN command_original TEXT")
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
        if (oldVersion < 4) addV4Columns(db)
        if (oldVersion < 5) addV5(db)
    }

    /** The new row's id, or null when [retention] saves nothing (or the insert failed). */
    fun add(row: HistoryRow, retention: Retention): Long? {
        if (!retention.stores) return null
        val id = writableDatabase.insert("history", null, values(row))
        purge(retention, row.createdAt)
        return id.takeIf { it > 0 }
    }

    private fun values(row: HistoryRow) = ContentValues().apply {
        put("created_at", row.createdAt); put("app_id", row.appId); put("status", row.status)
        put("asr_text", row.asrText); put("final_text", row.finalText)
        put("inserted", if (row.inserted) 1 else 0); put("audio_ms", row.audioMs)
        put("fallback_reason", row.fallbackReason); put("asr_ms", row.asrMs); put("llm_ms", row.llmMs); put("total_ms", row.totalMs)
        put("insert_outcome", row.insertOutcome); put("route", row.route)
        put("command_original", row.commandOriginal)
        put("app_category", row.appCategory)
    }

    fun purge(retention: Retention, now: Long = System.currentTimeMillis()) {
        val cutoff = retention.cutoff(now) ?: return
        if (writableDatabase.delete("history", "created_at < ?", arrayOf(cutoff.toString())) > 0) {
            dropOrphans()
            scrub(vacuum = retention == Retention.NEVER)
        }
    }

    fun delete(id: Long) {
        writableDatabase.delete("history", "id = ?", arrayOf(id.toString()))
        dropOrphans()
        scrub()
    }

    /** Versions and recordings whose row is gone (deleted, expired, cleared, replaced). */
    private fun dropOrphans() {
        writableDatabase.delete("history_versions", "history_id NOT IN (SELECT id FROM history)", null)
        runCatching { audio.retain(audio.ids().intersect(ids().toSet())) }
    }

    /** Every row id, newest first. */
    private fun ids(): List<Long> =
        readableDatabase.rawQuery("SELECT id FROM history ORDER BY created_at DESC, id DESC", null).use { c ->
            buildList { while (c.moveToNext()) add(c.getLong(0)) }
        }

    /** D35: keep [pcm] as row [id]'s recording, then only the newest [keep] rows keep theirs. */
    fun recordAudio(id: Long, pcm: ShortArray, keep: Int) {
        if (keep <= 0 || pcm.isEmpty()) return
        audio.save(id, pcm)
        pruneAudio(keep)
    }

    /** D35: only the newest [keep] rows keep their recording (0 = none). */
    fun pruneAudio(keep: Int) {
        val ids = RetryLogic.keepIds(ids(), keep)
        audio.retain(audio.ids().intersect(ids))
    }

    fun hasAudio(id: Long): Boolean = audio.has(id)

    /** Ids among [ids] whose recording is kept. */
    fun withAudio(ids: Collection<Long>): Set<Long> = audio.ids().intersect(ids.toSet())

    fun row(id: Long): HistoryRow? = rows("created_at DESC", null, "id = ?", arrayOf(id.toString())).firstOrNull()

    /** Every text of row [id], oldest first (empty for a row never retried). */
    fun versions(id: Long): List<HistoryVersion> = versionsFor(listOf(id))[id].orEmpty()

    fun versionsFor(ids: Collection<Long>): Map<Long, List<HistoryVersion>> {
        if (ids.isEmpty()) return emptyMap()
        val out = HashMap<Long, MutableList<HistoryVersion>>()
        for (chunk in ids.chunked(500)) {
            val q = chunk.joinToString(",") { "?" }
            readableDatabase.rawQuery(
                "SELECT history_id, n, created_at, status, asr_text, final_text, route FROM history_versions WHERE history_id IN ($q) ORDER BY history_id, n",
                chunk.map { it.toString() }.toTypedArray(),
            ).use { c ->
                while (c.moveToNext()) out.getOrPut(c.getLong(0)) { mutableListOf() } +=
                    HistoryVersion(c.getInt(1), c.getLong(2), c.getString(3), c.getString(4), c.getString(5), c.getString(6))
            }
        }
        return out
    }

    /**
     * D35: row [id] was retried and got [r]. Version 0 keeps the text as it was (first retry
     * only), the new text becomes the next version and the row's text. Returns the row as it
     * was and the new version's number, or null when the row is gone.
     */
    fun applyRetry(id: Long, r: dev.murmur.core.CommitResult, now: Long = System.currentTimeMillis()): Pair<HistoryRow, Int>? =
        addVersion(id, now) { RetryLogic.retried(it, r) }

    /**
     * D68: the user corrected row [id]'s text to [text]. Kept like a retry (D35): version 0 holds
     * the text as it was (first change only), the correction becomes the next version and the
     * row's text. Returns the row as it was, or null when the row is gone or [text] is no change.
     */
    fun applyCorrection(id: Long, text: String, now: Long = System.currentTimeMillis()): HistoryRow? =
        addVersion(id, now) { old -> old.copy(finalText = text).takeIf { text.isNotBlank() && text != old.finalText } }?.first

    /** [applyRetry] / [applyCorrection]: [next] turns the row into its new version (null = no change). */
    private fun addVersion(id: Long, now: Long, next: (HistoryRow) -> HistoryRow?): Pair<HistoryRow, Int>? {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val old = row(id) ?: return null
            val new = next(old) ?: return null
            val vs = versions(id)
            if (vs.isEmpty()) db.insertOrThrow("history_versions", null, versionValues(id, RetryLogic.original(old)))
            val n = (vs.maxOfOrNull { it.n } ?: 0) + 1
            db.insertOrThrow("history_versions", null, versionValues(id, HistoryVersion(n, now, new.status, new.asrText, new.finalText, new.route)))
            db.update("history", values(new), "id = ?", arrayOf(id.toString()))
            db.setTransactionSuccessful()
            return old to n
        } finally {
            db.endTransaction()
        }
    }

    private fun versionValues(id: Long, v: HistoryVersion) = ContentValues().apply {
        put("history_id", id); put("n", v.n); put("created_at", v.createdAt); put("status", v.status)
        put("asr_text", v.asrText); put("final_text", v.finalText); put("route", v.route)
    }

    fun recent(limit: Int = 500): List<HistoryRow> = rows("created_at DESC", limit.toString())

    /** Every saved row, oldest first (backup export). */
    fun all(): List<HistoryRow> = rows("created_at ASC, id ASC", null)

    private fun rows(order: String, limit: String?, where: String? = null, args: Array<String>? = null): List<HistoryRow> =
        readableDatabase.query("history", null, where, args, null, null, order, limit).use { c ->
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
                        commandOriginal = c.getString(c.getColumnIndexOrThrow("command_original")),
                        appCategory = c.getString(c.getColumnIndexOrThrow("app_category")),
                    ),
                )
            }
        }

    /**
     * Backup restore: every row replaced by [rows] in one transaction (a failure keeps the old
     * ones), then [retention] applied as after any insert.
     */
    fun replaceAll(rows: List<HistoryRow>, retention: Retention, now: Long = System.currentTimeMillis()) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            db.delete("history", null, null)
            db.delete("history_versions", null, null)
            for (r in rows) db.insertOrThrow("history", null, values(r))
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
        runCatching { audio.clear() } // restored rows get new ids: no recording belongs to them
        scrub(vacuum = true)
        purge(retention, now)
    }

    /** Every saved dictation, oldest first, for the one-time Insights backfill (failed ones and Command Mode left out). */
    fun statsRows(): List<InsightsStore.Row> =
        readableDatabase.query("history", arrayOf("created_at", "app_id", "final_text", "audio_ms"), "status NOT IN ('ERROR', 'COMMAND')", null, null, null, "created_at ASC").use { c ->
            buildList { while (c.moveToNext()) add(InsightsStore.Row(c.getLong(0), c.getString(1), c.getString(2), c.getLong(3))) }
        }

    fun clear() {
        writableDatabase.delete("history", null, null)
        writableDatabase.delete("history_versions", null, null)
        runCatching { audio.clear() }
        scrub(vacuum = true)
    }
}
