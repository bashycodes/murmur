package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CleanupLevel

/**
 * Main Settings is a hub with focused sub-pages (ui/SettingsActivity), like Developer settings.
 *
 * Save rule (one for the whole app, matching Developer settings):
 *  * a page with typed fields (Timing & limits, Excluded apps) has Save in the top bar, enabled
 *    only when something changed and every field validates; leaving with unsaved edits asks
 *    "Discard changes?" ([SettingsDraft]);
 *  * a page of switches and choices applies each change at once (Developer → Debug log & audio
 *    and Parakeet decoding work the same way); [effects] says what a change triggers.
 * Either way the whole [Settings] is written (the language / endpoint migrations rely on that),
 * starting from what is stored now, so a page only ever changes its own fields.
 */
object SettingsPages {
    /** [STYLE] = Writing style (D27); [APP_STYLES] = its per-app bucket list, opened from [STYLE] (not a hub row). */
    enum class Page { HUB, LANGUAGE, STYLE, APP_STYLES, ENGINE, CLEANUP, TIMING, BUBBLE, APPS, HISTORY }

    enum class Field { ENDPOINT, CAP, DENYLIST, APP_STYLES }

    /** `remote` builds have no on-device engine: no model import, no local LLM (hybrid-architecture §1.4). */
    fun visible(p: Page, caps: BuildCaps): Boolean = when (p) {
        Page.ENGINE, Page.CLEANUP -> caps.local
        else -> true
    }

    fun visiblePages(caps: BuildCaps): List<Page> = Page.entries.filter { it != Page.HUB && visible(it, caps) }

    fun hasForm(p: Page) = p == Page.TIMING || p == Page.APPS || p == Page.APP_STYLES

    fun pageOf(f: Field): Page = when (f) { Field.DENYLIST -> Page.APPS; Field.APP_STYLES -> Page.APP_STYLES; else -> Page.TIMING }

    val TITLES = mapOf(
        Page.HUB to "Settings", Page.LANGUAGE to "Language", Page.STYLE to "Writing style", Page.APP_STYLES to "Style per app", Page.ENGINE to "Speech engine", Page.CLEANUP to "AI cleanup",
        Page.TIMING to "Timing & limits", Page.BUBBLE to "Bubble", Page.APPS to "Excluded apps", Page.HISTORY to "History",
    )

    /** Whisper language codes offered in the picker; [Settings.LANGUAGE_UNSET] first. */
    val LANGUAGES = listOf(
        "auto" to "Auto-detect", "en" to "English", "es" to "Spanish", "fr" to "French", "de" to "German",
        "it" to "Italian", "pt" to "Portuguese", "nl" to "Dutch", "he" to "Hebrew", "ar" to "Arabic",
        "ru" to "Russian", "uk" to "Ukrainian", "pl" to "Polish", "tr" to "Turkish", "hi" to "Hindi",
        "zh" to "Chinese", "ja" to "Japanese", "ko" to "Korean",
    )

    /** Picker entries; a stored code not in the short list (set elsewhere) is shown as an extra entry and kept. */
    fun languageChoices(stored: String, phoneLanguage: String): List<Pair<String, String>> {
        val all = listOf(Settings.LANGUAGE_UNSET to "Phone language ($phoneLanguage)") + LANGUAGES
        return if (all.any { it.first == stored }) all else all + (stored to LanguageDefault.name(stored))
    }

    val LEVELS = listOf(
        CleanupLevel.LIGHT to "Light", CleanupLevel.MEDIUM to "Medium", CleanupLevel.HIGH to "High",
    )

    /** Stored NONE (never picked) runs as LIGHT when AI cleanup is on, so the picker shows LIGHT. */
    fun shownStrength(stored: CleanupLevel) = stored.takeIf { it != CleanupLevel.NONE } ?: CleanupLevel.LIGHT

    data class Effects(val reloadEngine: Boolean, val purgeHistory: Boolean)

    /** What saving [new] over [old] triggers (same as the old single page's Save). */
    fun effects(old: Settings, new: Settings) = Effects(
        reloadEngine = new.mockEngine != old.mockEngine || new.llmEnabled != old.llmEnabled || new.asrEngine != old.asrEngine,
        purgeHistory = new.retention != old.retention,
    )

    private fun strictness(r: Retention) = when (r) { Retention.KEEP -> 0; Retention.DAY -> 1; Retention.NEVER -> 2 }

    /** A retention change that deletes saved dictations right away: confirm first. */
    fun deletesHistory(old: Retention, new: Retention) = strictness(new) > strictness(old)

    private val PACKAGE = Regex("""[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+""")

    /** The form pages' inputs as typed. */
    data class Input(val endpointMs: String, val capMinutes: String, val decodeWhileSpeaking: Boolean, val denylist: String, val appStyles: String = "") {
        companion object {
            fun of(s: Settings) = Input(
                endpointMs = s.endpointSilenceMs.toString(), capMinutes = s.capMinutes.toString(),
                decodeWhileSpeaking = s.decodeWhileSpeaking, denylist = s.userDenylist.sorted().joinToString("\n"),
                appStyles = WritingStyle.formatOverrides(s.appBuckets),
            )
        }
    }

    /** [settings] = [base] with [p]'s fields from [i], or null while any error is shown. */
    data class Result(val errors: Map<Field, String>, val settings: Settings?)

    fun validate(base: Settings, i: Input, p: Page): Result {
        val errors = LinkedHashMap<Field, String>()
        var out = base
        if (p == Page.TIMING) {
            val e = i.endpointMs.trim()
            val ms = if (e.isEmpty()) 0 else e.toIntOrNull()
            val r = Settings.ENDPOINT_RANGE_MS
            if (ms == null || (ms != 0 && ms !in r)) errors[Field.ENDPOINT] = "Enter ${r.first}–${r.last} ms, or 0 to stop only when you tap"
            val cap = i.capMinutes.trim().toIntOrNull()
            if (cap == null || cap !in 1..20) errors[Field.CAP] = "Enter 1–20 minutes"
            if (errors.isEmpty()) out = out.copy(endpointSilenceMs = ms!!, capMinutes = cap!!, decodeWhileSpeaking = i.decodeWhileSpeaking)
        }
        if (p == Page.APPS) {
            val lines = i.denylist.lines().map { it.trim() }.filter { it.isNotEmpty() }
            val bad = lines.filter { !PACKAGE.matches(it) }
            if (bad.isNotEmpty()) errors[Field.DENYLIST] = "Not a package name: " + bad.distinct().joinToString(", ") + " (e.g. com.example.app)"
            else out = out.copy(userDenylist = lines.toSet())
        }
        if (p == Page.APP_STYLES) {
            val parsed = WritingStyle.parseOverrides(i.appStyles)
            if (parsed.errors.isNotEmpty()) {
                errors[Field.APP_STYLES] = "Write package = personal, work, email or other: " + parsed.errors.distinct().joinToString(", ")
            } else out = out.copy(appBuckets = parsed.map)
        }
        return Result(errors, if (errors.isEmpty()) out else null)
    }
}

/** One form page's edit state: what is stored ([saved]) and what the page shows ([draft]). */
data class SettingsDraft(val base: Settings, val saved: SettingsPages.Input, val draft: SettingsPages.Input, val page: SettingsPages.Page) {
    val dirty: Boolean get() = when (page) {
        SettingsPages.Page.TIMING -> draft.copy(denylist = "", appStyles = "") != saved.copy(denylist = "", appStyles = "")
        SettingsPages.Page.APPS -> draft.denylist != saved.denylist
        SettingsPages.Page.APP_STYLES -> draft.appStyles != saved.appStyles
        else -> false
    }

    fun edit(f: SettingsPages.Input.() -> SettingsPages.Input) = copy(draft = draft.f())

    /** Validated against the store as it is now ([stored]), so Save never undoes another page. */
    fun result(stored: Settings = base) = SettingsPages.validate(stored, draft, page)

    fun canSave(stored: Settings = base) = dirty && result(stored).settings != null

    /** The store changed under the page: keep this page's edits, take the new stored values. */
    fun rebase(stored: Settings) = copy(base = stored, saved = SettingsPages.Input.of(stored))

    companion object {
        fun of(s: Settings, page: SettingsPages.Page) = SettingsPages.Input.of(s).let { SettingsDraft(s, it, it, page) }
    }
}

/** Hub row subtitles and the status card: the current value at a glance. */
object SettingsSummary {
    const val WARN_MOCK = "Mock engine is on: every dictation inserts a fixed test sentence."
    const val WARN_NO_WHISPER = "No speech model imported: import one under Speech engine."
    const val WARN_NO_PARAKEET = "Parakeet is selected but no model folder is imported: whisper is used."
    const val WARN_NO_LLM = "AI cleanup is on but no GGUF model is imported: plain transcripts are inserted."

    private val PREFIXES = listOf("sherpa-onnx-", "nemo-", "parakeet-tdt-", "parakeet-", "ggml-")
    private val SUFFIX = Regex("""(-instruct)?([-.](q\d\w*|int8|fp16|f16|f32))?$""", RegexOption.IGNORE_CASE)

    /** "sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8" → "unified-en-0.6b"; "qwen2.5-1.5b-instruct-q4_k_m.gguf" → "qwen2.5-1.5b". */
    fun shortModel(fileName: String?): String? {
        if (fileName.isNullOrEmpty()) return null
        var n = fileName.substringAfterLast('/').removeSuffix(".gguf").removeSuffix(".bin")
        var changed = true
        while (changed) {
            changed = false
            for (p in PREFIXES) if (n.startsWith(p, ignoreCase = true) && n.length > p.length) { n = n.substring(p.length); changed = true }
        }
        return n.replace(SUFFIX, "").ifEmpty { n }
    }

    fun language(s: Settings, effective: String): String {
        val l = if (s.language == Settings.LANGUAGE_UNSET) {
            "Phone language (" + (if (effective == "auto") "auto-detect" else LanguageDefault.name(effective)) + ")"
        } else LanguageDefault.name(s.language)
        return "$l · auto-format ${if (s.spokenCleanup) "on" else "off"}"
    }

    /** [whisper] / [parakeet]: imported file / folder names (null = none). */
    fun engine(s: Settings, whisper: String?, parakeet: String?): String = when (s.asrEngine) {
        AsrEngine.PARAKEET -> "Parakeet · " + (shortModel(parakeet) ?: "no model folder (whisper is used)")
        else -> "Whisper · " + (shortModel(whisper) ?: "no model imported")
    }

    fun levelName(l: CleanupLevel) = SettingsPages.LEVELS.firstOrNull { it.first == l }?.second ?: "Off"

    fun cleanup(s: Settings, llm: String?): String {
        // D27: the LLM switch no longer gates the deterministic formatting ("Auto-format" does).
        if (!s.llmEnabled) return if (s.spokenCleanup) "Off · auto-format still runs" else "Off · no formatting (auto-format off)"
        return "On · ${shortModel(llm) ?: "no model imported"} · ${levelName(s.effectiveCleanup)}"
    }

    private fun seconds(ms: Int): String {
        val v = ms / 1000.0
        return if (ms % 1000 == 0) "${ms / 1000}" else String.format(java.util.Locale.ROOT, "%.1f", v).trimEnd('0').trimEnd('.')
    }

    fun timing(s: Settings): String = listOfNotNull(
        if (s.endpointSilenceMs > 0) "Stops after ${seconds(s.endpointSilenceMs)} s of silence" else "Stops when you tap",
        if (s.decodeWhileSpeaking) "decode while speaking" else null,
        "${s.capMinutes} min limit",
    ).joinToString(" · ")

    fun bubble(s: Settings): String {
        val on = listOfNotNull(
            if (s.livePreview) "live preview" else null,
            if (s.shrinkBubble) "shrinks to a dot" else null,
            if (s.shakeToReveal) "shake to restore" else null,
        )
        return if (on.isEmpty()) "Preview, dot and shake off" else on.joinToString(" · ").replaceFirstChar { it.uppercase() }
    }

    fun apps(s: Settings): String = when (val n = s.userDenylist.size) {
        0 -> "None (banking apps are always excluded)"
        1 -> "1 app"
        else -> "$n apps"
    }

    fun history(r: Retention): String = when (r) {
        Retention.KEEP -> "Keep everything"
        Retention.DAY -> "Last 24 hours"
        Retention.NEVER -> "Never saved"
    }

    /** Hub status card warnings, in this order. Only what this build can actually run. */
    fun warnings(s: Settings, caps: BuildCaps, whisper: String?, parakeet: String?, llm: String?): List<String> = buildList {
        if (s.mockEngine) add(WARN_MOCK)
        if (!caps.local) return@buildList
        val parakeetOk = s.asrEngine == AsrEngine.PARAKEET && parakeet != null
        if (s.asrEngine == AsrEngine.PARAKEET && parakeet == null) add(WARN_NO_PARAKEET)
        // A hybrid phone can dictate through the server without any local model.
        if (!caps.remote && !parakeetOk && whisper == null) add(WARN_NO_WHISPER)
        if (s.llmEnabled && llm == null) add(WARN_NO_LLM)
    }
}
