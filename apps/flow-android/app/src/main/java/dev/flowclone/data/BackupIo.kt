package dev.flowclone.data

import android.content.Context
import android.content.SharedPreferences
import dev.flowclone.FlowApp

/**
 * Reads every user-data store into a [Backup.Data] and writes one back (Settings → Back up &
 * restore). Stores (all app-private):
 *  - `flow_settings` prefs ([SettingsStore]) minus [Backup.DEVICE_LOCAL]
 *  - `dev` prefs ([DevStore]) minus every secret ([Backup.isSecret]: server token, cloud keys)
 *  - `files/dictionary.json` ([DictionaryStore]: words and snippets)
 *  - `files/learn_suggestions.json` ([SuggestionStore])
 *  - `files/insights.json` ([InsightsStore])
 *  - `files/notes/` ([dev.flowclone.notes.NotesStore]: Notes and the Scratchpad / Quick note)
 *  - `history.db` ([HistoryStore])
 * Not backed up: models (`files/models`, `files/parakeet`, `files/llm`), the debug log and debug
 * audio (`files/debug/`), Keystore keys.
 *
 * A restore replaces each section the file has; secrets and device-local values already on this
 * install are kept (so a restore over a working setup does not lose its keys).
 */
class BackupIo(private val app: FlowApp) {
    private fun prefs(name: String): SharedPreferences = app.getSharedPreferences(name, Context.MODE_PRIVATE)

    fun collect(): Backup.Data = Backup.Data(
        settings = snapshot(Backup.SETTINGS_PREFS),
        developer = snapshot(Backup.DEV_PREFS),
        dictionary = app.dictionary.all(),
        suggestions = app.suggestions.load(),
        insights = app.insights.load(),
        notes = app.documentsIo.submit<List<dev.flowclone.notes.Note>> { app.documents.list() }.get(),
        history = app.history.all(),
    )

    fun export(now: java.time.ZonedDateTime = java.time.ZonedDateTime.now()): String = Backup.encode(
        Backup.Meta(
            appVersion = AppVersion.current.version,
            platform = "android",
            flavor = app.caps.flavor,
            created = now.withNano(0).format(java.time.format.DateTimeFormatter.ISO_OFFSET_DATE_TIME),
        ),
        collect(),
    )

    private fun snapshot(name: String): Map<String, Backup.Pref> =
        prefs(name).all.mapNotNull { (k, v) -> if (Backup.exportable(name, k)) Backup.Pref.of(v)?.let { k to it } else null }.toMap()

    /**
     * Writes [d] over the app's data. [d] comes from [Backup.decode], so it is fully parsed and
     * validated; the order is the file stores first (each atomic on its own), settings last.
     */
    fun restore(d: Backup.Data) {
        d.dictionary?.let { app.dictionary.replaceAll(it) }
        d.suggestions?.let { s -> app.suggestions.update { s } }
        d.notes?.let { n -> app.documentsIo.submit { app.documents.replaceAll(n) }.get() }
        d.settings?.let { replacePrefs(Backup.SETTINGS_PREFS, it) }
        d.developer?.let { replacePrefs(Backup.DEV_PREFS, it) }
        d.history?.let { app.history.replaceAll(it, app.settings.load().retention) }
        // After History: an insights file without `backfilled` must not count the rows again.
        d.insights?.let { app.insights.replace(it) }
        // Refresh what is cached from prefs (the bubble's flows), then whatever the settings drive.
        app.settings.save(app.settings.load())
        app.migrateCloud()
        app.engine.reload()
    }

    /** Clears [name] except its secrets / device-local values, then writes [m]; one commit. */
    private fun replacePrefs(name: String, m: Map<String, Backup.Pref>) {
        val sp = prefs(name)
        val e = sp.edit().clear()
        for ((k, v) in sp.all) if (!Backup.exportable(name, k)) put(e, k, Backup.Pref.of(v) ?: continue)
        for ((k, v) in m) if (Backup.exportable(name, k)) put(e, k, v)
        if (!e.commit()) throw java.io.IOException("could not save settings")
    }

    private fun put(e: SharedPreferences.Editor, k: String, p: Backup.Pref) {
        when (p) {
            is Backup.Pref.Str -> e.putString(k, p.v)
            is Backup.Pref.Bool -> e.putBoolean(k, p.v)
            is Backup.Pref.Int32 -> e.putInt(k, p.v)
            is Backup.Pref.Int64 -> e.putLong(k, p.v)
            is Backup.Pref.F32 -> e.putFloat(k, p.v)
            is Backup.Pref.StrSet -> e.putStringSet(k, p.v)
        }
    }
}
