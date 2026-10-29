package dev.flowclone.data

import dev.flowclone.core.CoreJson
import dev.flowclone.core.DictEntry
import dev.flowclone.learn.Suggestions
import dev.flowclone.notes.DocumentStore
import dev.flowclone.notes.Note
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * "Back up & restore" (Settings): every piece of user data in one versioned JSON file, so an
 * install that wipes app data (uninstall, a signing-key change) can get it back. The file format
 * is `docs/backup-format.md` (the Windows app is meant to reuse it). Pure: the Android side that
 * reads and writes the stores is [BackupIo].
 *
 * Never in a backup: API keys, the server token or anything else sealed by the Keystore
 * ([isSecret], checked on export and again on import), downloaded models (paths to them are
 * device-local, [DEVICE_LOCAL]), the debug log and debug audio.
 */
object Backup {
    const val FORMAT = "flow-clone-backup"
    const val VERSION = 1

    /** Larger than any real backup (text only); a bigger file is refused before parsing. */
    const val MAX_BYTES = 64 * 1024 * 1024

    /** Shown under the Settings rows and in the restore dialog. */
    const val SECRETS_NOTE =
        "API keys and the server token are never in a backup: enter them again after restoring. Downloaded models are not included either."

    /** SharedPreferences file names, as the stores open them. */
    const val SETTINGS_PREFS = "flow_settings"
    const val DEV_PREFS = DevStore.FILE

    /**
     * Keys that hold a secret or its ciphertext: the server token (`server.token_sealed`), cloud
     * API keys (`cloud.key_sealed.<id>`) and, defensively, anything named like one.
     */
    private val SECRET = Regex("""token|secret|password|passwd|key_sealed|api_?key|credential""", RegexOption.IGNORE_CASE)

    fun isSecret(key: String): Boolean = SECRET.containsMatchIn(key) || key.startsWith(CloudKeyStore.PREFIX)

    /** `flow_settings` keys that only mean something on this install: imported model files, the service's stop flag. */
    val DEVICE_LOCAL = setOf("model_path", "parakeet_path", "llm_model_path", "service_stopped_by_user")

    /** Whether [key] of prefs file [prefs] goes into (or comes out of) a backup. */
    fun exportable(prefs: String, key: String): Boolean =
        !isSecret(key) && !(prefs == SETTINGS_PREFS && key in DEVICE_LOCAL)

    /** `flow-clone-backup-2026-10-29.json` */
    fun fileName(date: java.time.LocalDate): String = "flow-clone-backup-$date.json"

    /** One SharedPreferences value with its type, so a restore writes it back exactly. */
    sealed class Pref {
        data class Str(val v: String) : Pref()
        data class Bool(val v: Boolean) : Pref()
        data class Int32(val v: Int) : Pref()
        data class Int64(val v: Long) : Pref()
        data class F32(val v: Float) : Pref()
        data class StrSet(val v: Set<String>) : Pref()

        companion object {
            /** A `SharedPreferences.getAll()` value; null for anything else. */
            fun of(v: Any?): Pref? = when (v) {
                is String -> Str(v)
                is Boolean -> Bool(v)
                is Int -> Int32(v)
                is Long -> Int64(v)
                is Float -> F32(v)
                is Set<*> -> StrSet(v.filterIsInstance<String>().toSet())
                else -> null
            }
        }
    }

    /**
     * What a backup holds. A null section was not in the file and is left alone on restore;
     * a present one (even empty) replaces what the app has.
     */
    data class Data(
        val settings: Map<String, Pref>? = null,
        val developer: Map<String, Pref>? = null,
        val dictionary: List<DictEntry>? = null,
        val suggestions: Suggestions? = null,
        val insights: Insights.Aggregates? = null,
        val notes: List<Note>? = null,
        val history: List<HistoryRow>? = null,
    )

    data class Meta(val appVersion: String, val platform: String, val flavor: String, val created: String)

    sealed class Parsed {
        data class Ok(val meta: Meta, val data: Data) : Parsed()
        data class Rejected(val reason: String) : Parsed()
    }

    // ---- encode ----

    fun encode(meta: Meta, d: Data): String {
        val data = JSONObject()
        d.settings?.let { data.put("settings", prefsJson(SETTINGS_PREFS, it)) }
        d.developer?.let { data.put("developer", prefsJson(DEV_PREFS, it)) }
        d.dictionary?.let { data.put("dictionary", JSONArray(CoreJson.encodeDictionary(it))) }
        d.suggestions?.let { data.put("learn_suggestions", JSONObject(Suggestions.encode(it))) }
        d.insights?.let { data.put("insights", it.toJson()) }
        d.notes?.let { ns ->
            data.put("notes", JSONArray().apply {
                for (n in ns) put(JSONObject().put("id", n.id).put("text", n.text).put("pinned", n.pinned).put("created", n.createdAt).put("updated", n.updatedAt))
            })
        }
        d.history?.let { rows -> data.put("history", JSONArray().apply { for (r in rows) put(historyJson(r)) }) }
        return JSONObject()
            .put("format", FORMAT)
            .put("version", VERSION)
            .put("app_version", meta.appVersion)
            .put("platform", meta.platform)
            .put("flavor", meta.flavor)
            .put("created", meta.created)
            .put("data", data)
            .toString(1)
    }

    private fun prefsJson(file: String, m: Map<String, Pref>) = JSONObject().apply {
        for ((k, p) in m.toSortedMap()) {
            if (!exportable(file, k)) continue
            val (t, v) = when (p) {
                is Pref.Str -> "string" to p.v
                is Pref.Bool -> "bool" to p.v
                is Pref.Int32 -> "int" to p.v
                is Pref.Int64 -> "long" to p.v
                is Pref.F32 -> "float" to p.v.toDouble()
                is Pref.StrSet -> "string_set" to JSONArray(p.v.sorted())
            }
            put(k, JSONObject().put("type", t).put("value", v))
        }
    }

    private fun historyJson(r: HistoryRow) = JSONObject()
        .put("created_at", r.createdAt).put("app_id", r.appId ?: JSONObject.NULL).put("status", r.status)
        .put("asr_text", r.asrText).put("final_text", r.finalText).put("inserted", r.inserted).put("audio_ms", r.audioMs)
        .put("fallback_reason", r.fallbackReason ?: JSONObject.NULL)
        .put("asr_ms", r.asrMs).put("llm_ms", r.llmMs).put("total_ms", r.totalMs)
        .put("insert_outcome", r.insertOutcome ?: JSONObject.NULL).put("route", r.route ?: JSONObject.NULL)
        .put("command_original", r.commandOriginal ?: JSONObject.NULL)

    // ---- decode (everything is checked before anything is written) ----

    private class Bad(msg: String) : Exception(msg)

    fun decode(text: String): Parsed {
        val root = try {
            JSONObject(text.trimStart('﻿'))
        } catch (_: JSONException) {
            return Parsed.Rejected("This file is not a Flow Clone backup (not JSON).")
        }
        if (root.optString("format") != FORMAT) return Parsed.Rejected("This file is not a Flow Clone backup.")
        val v = root.opt("version")
        if (v !is Int || v < 1) return Parsed.Rejected("This backup is damaged (no valid format version).")
        if (v > VERSION) {
            return Parsed.Rejected(
                "This backup was made by a newer version of Flow Clone (backup format $v; this app reads up to $VERSION). Update the app, then restore.",
            )
        }
        return try {
            val d = root.optJSONObject("data") ?: throw Bad("no data section")
            val meta = Meta(
                appVersion = root.optString("app_version", "unknown"),
                platform = root.optString("platform", "unknown"),
                flavor = root.optString("flavor", ""),
                created = root.optString("created", ""),
            )
            Parsed.Ok(meta, decodeData(d))
        } catch (e: Bad) {
            Parsed.Rejected("This backup is damaged: ${e.message}.")
        } catch (e: JSONException) {
            Parsed.Rejected("This backup is damaged: ${e.message}")
        }
    }

    private fun decodeData(d: JSONObject) = Data(
        settings = d.section("settings")?.let { prefsOf(SETTINGS_PREFS, it.obj("settings")) },
        developer = d.section("developer")?.let { prefsOf(DEV_PREFS, it.obj("developer")) },
        dictionary = d.section("dictionary")?.let { CoreJson.decodeDictionary(it.arr("dictionary").toString()) },
        suggestions = d.section("learn_suggestions")?.let { Suggestions.decode(it.obj("learn_suggestions").toString()) },
        insights = d.section("insights")?.let { Insights.Aggregates.fromJson(it.obj("insights").toString()) },
        notes = d.section("notes")?.let { notesOf(it.arr("notes")) },
        history = d.section("history")?.let { historyOf(it.arr("history")) },
    )

    /** The section's value, or null when absent / JSON null. */
    private fun JSONObject.section(k: String): Any? = if (has(k) && !isNull(k)) get(k) else null

    private fun Any.obj(name: String) = this as? JSONObject ?: throw Bad("\"$name\" is not an object")
    private fun Any.arr(name: String) = this as? JSONArray ?: throw Bad("\"$name\" is not a list")

    private fun prefsOf(file: String, o: JSONObject): Map<String, Pref> = buildMap {
        for (k in o.keys()) {
            if (!exportable(file, k)) continue // a hand-made file can't plant a secret or a model path
            val e = o.optJSONObject(k) ?: throw Bad("setting \"$k\" is malformed")
            val p = try {
                when (val t = e.optString("type")) {
                    "string" -> Pref.Str(e.getString("value"))
                    "bool" -> Pref.Bool(e.getBoolean("value"))
                    "int" -> Pref.Int32(e.getInt("value"))
                    "long" -> Pref.Int64(e.getLong("value"))
                    "float" -> Pref.F32(e.getDouble("value").toFloat())
                    "string_set" -> e.getJSONArray("value").let { a -> Pref.StrSet((0 until a.length()).map { a.getString(it) }.toSet()) }
                    else -> throw Bad("setting \"$k\" has unknown type \"$t\"")
                }
            } catch (x: JSONException) {
                throw Bad("setting \"$k\" is malformed")
            }
            put(k, p)
        }
    }

    private fun notesOf(a: JSONArray): List<Note> {
        val seen = HashSet<String>()
        return (0 until a.length()).map { i ->
            val o = a.optJSONObject(i) ?: throw Bad("note ${i + 1} is malformed")
            val id = o.optString("id")
            if (!DocumentStore.isValidId(id)) throw Bad("note ${i + 1} has an invalid id")
            if (!seen.add(id)) throw Bad("note id \"$id\" appears twice")
            if (!o.has("text") || o.isNull("text")) throw Bad("note \"$id\" has no text")
            Note(id, o.getString("text"), o.optBoolean("pinned", false), o.optLong("created", 0), o.optLong("updated", 0))
        }
    }

    private fun historyOf(a: JSONArray): List<HistoryRow> = (0 until a.length()).map { i ->
        val o = a.optJSONObject(i) ?: throw Bad("history entry ${i + 1} is malformed")
        fun str(k: String) = if (!o.has(k) || o.isNull(k)) null else o.getString(k)
        try {
            HistoryRow(
                id = 0, createdAt = o.getLong("created_at"), appId = str("app_id"), status = o.getString("status"),
                asrText = o.optString("asr_text"), finalText = o.optString("final_text"), inserted = o.optBoolean("inserted"),
                audioMs = o.optLong("audio_ms"), fallbackReason = str("fallback_reason"),
                asrMs = o.optLong("asr_ms"), llmMs = o.optLong("llm_ms"), totalMs = o.optLong("total_ms"),
                insertOutcome = str("insert_outcome"), route = str("route"), commandOriginal = str("command_original"),
            )
        } catch (x: JSONException) {
            throw Bad("history entry ${i + 1} is malformed")
        }
    }

    // ---- what the confirm dialog shows ----

    /** "12 notes · 340 dictations · 25 dictionary words · 3 snippets · …" lines. */
    fun summary(meta: Meta, d: Data): List<String> = buildList {
        val created = meta.created.ifEmpty { "an unknown date" }
        add("Made $created by Flow Clone ${meta.appVersion} (${meta.platform}${if (meta.flavor.isNotEmpty()) ", ${meta.flavor}" else ""}).")
        d.notes?.let { add(count(it.size, "note")) }
        d.history?.let { add(count(it.size, "dictation") + " in History") }
        d.dictionary?.let { dict ->
            val snippets = dict.count { it.isSnippet }
            add(count(dict.size - snippets, "dictionary word") + " · " + count(snippets, "snippet"))
        }
        d.suggestions?.let { add(count(it.pending.size, "learned-word suggestion")) }
        d.insights?.let { i -> add("Insights for " + count(i.days.count { it.value.count > 0 }, "day")) }
        if (d.settings != null || d.developer != null) add("Settings (" + count((d.settings?.size ?: 0) + (d.developer?.size ?: 0), "value") + ")")
    }

    private fun count(n: Int, what: String) = "$n $what" + if (n == 1) "" else "s"
}
