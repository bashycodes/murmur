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
    /**
     * [STYLE] = Writing style (D27); [APP_STYLES] = its per-app bucket list, opened from [STYLE] (not a hub row).
     * [SPEECH] = Speech to text (AndroidSpeech.dc.html: ranked engines, the phone's model); [ENGINE]
     * = the on-device engine choice and model imports, opened from [SPEECH] (not a hub row).
     */
    enum class Page { HUB, LANGUAGE, STYLE, APP_STYLES, SPEECH, ENGINE, CLEANUP, TIMING, BUBBLE, APPS, HISTORY }

    enum class Field { ENDPOINT, CAP, DENYLIST }

    /** `remote` builds have no on-device engine: no model import, no local LLM (hybrid-architecture §1.4). */
    fun visible(p: Page, caps: BuildCaps): Boolean = when (p) {
        Page.ENGINE, Page.CLEANUP -> caps.local
        else -> true
    }

    fun visiblePages(caps: BuildCaps): List<Page> = Page.entries.filter { it != Page.HUB && visible(it, caps) }

    fun hasForm(p: Page) = p == Page.TIMING || p == Page.APPS

    fun pageOf(f: Field): Page = when (f) { Field.DENYLIST -> Page.APPS; else -> Page.TIMING }

    val TITLES = mapOf(
        Page.HUB to "Settings", Page.LANGUAGE to "Language", Page.STYLE to "Writing style", Page.APP_STYLES to "Style per app", Page.SPEECH to "Speech to text",
        Page.ENGINE to "Model on this phone", Page.CLEANUP to "AI cleanup",
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

    /** The form pages' inputs as typed. ("Transcribe while I speak" is a switch on [Page.SPEECH] now.) */
    data class Input(val endpointMs: String, val capMinutes: String, val denylist: String) {
        companion object {
            fun of(s: Settings) = Input(
                endpointMs = s.endpointSilenceMs.toString(), capMinutes = s.capMinutes.toString(),
                denylist = s.userDenylist.sorted().joinToString("\n"),
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
            if (errors.isEmpty()) out = out.copy(endpointSilenceMs = ms!!, capMinutes = cap!!)
        }
        if (p == Page.APPS) {
            val lines = i.denylist.lines().map { it.trim() }.filter { it.isNotEmpty() }
            val bad = lines.filter { !PACKAGE.matches(it) }
            if (bad.isNotEmpty()) errors[Field.DENYLIST] = "Not a package name: " + bad.distinct().joinToString(", ") + " (e.g. com.example.app)"
            else out = out.copy(userDenylist = lines.toSet())
        }
        return Result(errors, if (errors.isEmpty()) out else null)
    }
}

/** One form page's edit state: what is stored ([saved]) and what the page shows ([draft]). */
data class SettingsDraft(val base: Settings, val saved: SettingsPages.Input, val draft: SettingsPages.Input, val page: SettingsPages.Page) {
    val dirty: Boolean get() = when (page) {
        SettingsPages.Page.TIMING -> draft.copy(denylist = "") != saved.copy(denylist = "")
        SettingsPages.Page.APPS -> draft.denylist != saved.denylist
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
    const val WARN_NO_LLM = "AI cleanup is on but no GGUF model is imported: formatted transcripts are inserted without it."

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

    /** [cloud]: D28 cloud cleanup line ("OpenRouter (llama-3.3-70b-instruct), then on-device"), null when off. */
    fun cleanup(s: Settings, llm: String?, cloud: String? = null): String {
        if (cloud != null) return "$cloud · ${levelName(s.cleanupFor(true))}"
        // D27: the LLM switch no longer gates the deterministic formatting ("Auto-format" does).
        if (!s.llmEnabled) return if (s.spokenCleanup) "Off · auto-format still runs" else "Off · no formatting (auto-format off)"
        return "On · ${shortModel(llm) ?: "no model imported"} · ${levelName(s.effectiveCleanup)}"
    }

    private fun seconds(ms: Int): String {
        val v = ms / 1000.0
        return if (ms % 1000 == 0) "${ms / 1000}" else String.format(java.util.Locale.ROOT, "%.1f", v).trimEnd('0').trimEnd('.')
    }

    /** Timing and limits row (AndroidSettings.dc.html): "Stops after a 2 s pause · 5 min max". */
    fun timing(s: Settings): String =
        (if (s.endpointSilenceMs > 0) "Stops after a ${seconds(s.endpointSilenceMs)} s pause" else "Stops when you tap") + " · ${s.capMinutes} min max"

    /** Bubble size row: the size first ("100%"), then what else the Bubble page has on. */
    fun bubble(s: Settings): String = listOfNotNull(
        "${bubbleSizePercent(s.bubbleScale)}%",
        if (s.livePreview) "live preview" else null,
        if (!s.shrinkBubble) null else if (s.shrinkToDot) "shrinks to a dot" else "shrinks when idle",
        if (s.shakeToReveal) "shake to restore" else null,
    ).joinToString(" · ")

    /** The "Bubble size" slider's value: 75..150 in steps of 5. */
    fun bubbleSizePercent(scale: Float): Int = kotlin.math.round(Settings.bubbleScaleOf(scale) * 100f).toInt()

    /** Excluded apps row: the built-in banking list is always on ("Banking apps and 2 more"). */
    fun apps(s: Settings): String = when (val n = s.userDenylist.size) {
        0 -> "Banking apps"
        else -> "Banking apps and $n more"
    }

    /** History's "Keep dictations" row (History page, Home). */
    fun history(r: Retention): String = when (r) {
        Retention.KEEP -> "Keep everything"
        Retention.DAY -> "Last 24 hours"
        Retention.NEVER -> "Never saved"
    }

    /** Settings hub "Keep history" row ("1 day. Stored only on this phone."). */
    fun keepHistory(r: Retention): String = when (r) {
        Retention.KEEP -> "Until you delete it. Stored only on this phone."
        Retention.DAY -> "1 day. Stored only on this phone."
        Retention.NEVER -> "Off. Nothing is saved."
    }

    const val DEBUG_LOG_DESC = "Saves what you said and timings for bug reports. Never uploaded."

    /** An engine's name on the Speech to text page and in summaries. */
    fun engineName(r: DevSettings.Route): String = when (r) {
        DevSettings.Route.SERVER -> "Your server"
        DevSettings.Route.CLIENT -> "This phone"
        DevSettings.Route.CLOUD -> OpenRouter.NAME
    }

    /** Speech to text row: the engines that are on, in try order ("Your server, then this phone"). */
    fun speech(d: DevSettings, caps: BuildCaps): String {
        val on = DevPages.engines(d, caps).filter { DevPages.engineOn(d.effective(caps), it) }
        if (on.isEmpty()) return "Off: no engine is on"
        if (on.size == 1) return "${engineName(on[0])} only"
        return on.mapIndexed { i, r -> if (i == 0) engineName(r) else engineName(r).replaceFirstChar { it.lowercase() } }.joinToString(", then ")
    }

    /**
     * AI cleanup row: strength, then the engines in the order they are tried, then the formatting
     * rules ("Medium · OpenRouter, then rules"). [d] = the effective DevSettings (cloud cleanup).
     */
    fun cleanupRow(s: Settings, d: DevSettings, caps: BuildCaps): String {
        val cloud = d.cloudCleanupRouted && d.cloud.cleanup.consentOk
        val phone = s.llmEnabled && caps.local
        val engines = if (cloud) d.cleanupOrder.mapNotNull {
            when (it) {
                DevSettings.CleanupRoute.CLOUD -> OpenRouter.NAME
                DevSettings.CleanupRoute.LOCAL -> if (phone) "this phone" else null
            }
        } else if (phone) listOf("this phone") else emptyList()
        val chain = engines + listOfNotNull(if (s.spokenCleanup) "rules" else null)
        if (engines.isEmpty()) return if (s.spokenCleanup) "Rules only" else "Off"
        return levelName(s.cleanupFor(cloud)) + " · " + chain.mapIndexed { i, e -> if (i == 0) e.replaceFirstChar { it.uppercase() } else e }.joinToString(", then ")
    }

    /**
     * "Your server" row from the router's last health probe ([h], null = none; [checking] while one runs):
     * "Connected · 41 ms" / "Not reachable" / "Not set up" / "Off".
     */
    fun server(d: DevSettings, caps: BuildCaps, h: dev.flowclone.core.HealthReport?, checking: Boolean = false): String = when {
        !caps.remote -> "Not in this build"
        !d.serverConfigured -> "Not set up"
        !d.allowServer -> "Off · ${d.host}"
        h == null -> if (checking) "Checking…" else "Not checked yet"
        h.ok -> "Connected" + (h.rttMs?.let { " · ${it.median} ms" } ?: "")
        else -> "Not reachable"
    }

    /** OpenRouter row: "Key saved · $0.42 this month" (usage counted on this phone only). */
    fun openRouter(hasKey: Boolean, usage: CloudUsage): String =
        (if (hasKey) "Key saved" else "No key") + " · ${CloudText.usd(usage.totalCostUsd)} this month"

    /** "This phone" engine line: the on-device engine that runs ("Parakeet · works offline"). */
    fun phoneEngine(s: Settings, whisper: String?, parakeet: String?): String = when {
        s.mockEngine -> "Mock engine (test sentence)"
        s.asrEngine == AsrEngine.PARAKEET && parakeet != null -> "Parakeet · works offline"
        whisper != null -> "Whisper · works offline"
        else -> "No model imported"
    }

    /** OpenRouter engine line: "Sends audio · asks first" until consent, then its model. */
    fun cloudEngine(d: DevSettings, hasKey: Boolean): String =
        "Sends audio · " + (if (!d.cloud.stt.consentOk) "asks first" else d.cloud.stt.model.substringAfter('/')) +
            if (d.allowCloud && !hasKey) " · no key" else ""

    /** "English" for Parakeet and *.en whisper models, else "Many languages". */
    fun modelLanguage(engine: AsrEngine, fileName: String): String =
        if (engine == AsrEngine.PARAKEET || Regex("""[.-]en([.-]|$)""").containsMatchIn(fileName.substringAfterLast('/'))) "English" else "Many languages"

    /** The phone's model card: "English · 640 MB · imported Oct 20" ([date] formats the file's time). */
    fun modelLine(engine: AsrEngine, fileName: String, bytes: Long, modifiedMs: Long, date: (Long) -> String): String =
        listOfNotNull(modelLanguage(engine, fileName), ModelPages.size(bytes), if (modifiedMs > 0) "imported ${date(modifiedMs)}" else null).joinToString(" · ")

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
