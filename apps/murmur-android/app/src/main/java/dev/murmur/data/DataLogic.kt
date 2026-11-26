package dev.murmur.data

import dev.murmur.core.CleanupLevel
import dev.murmur.core.DictEntry
import dev.murmur.core.FieldContext
import dev.murmur.core.SessionConfig
import dev.murmur.policy.AppCategorizer
import dev.murmur.policy.BubbleVisibilityPolicy
import dev.murmur.policy.EditorInputType
import dev.murmur.session.SessionCap

/** History retention pref: keep / 24 h / never. */
enum class Retention(val key: String) {
    KEEP("keep"), DAY("day"), NEVER("never");

    val stores: Boolean get() = this != NEVER

    /** Rows older than this (epoch ms) are purged; null = keep everything. */
    fun cutoff(now: Long): Long? = when (this) {
        KEEP -> null
        DAY -> now - 86_400_000L
        NEVER -> Long.MAX_VALUE
    }

    companion object {
        fun fromKey(k: String?): Retention = entries.firstOrNull { it.key == k } ?: KEEP
    }
}

/**
 * Light / dark for the app's screens (Settings → Theme). Dark when nothing is stored: a fresh
 * install starts dark; an install from before the setting keeps System ([ThemeDefault], D104).
 * The bubble is always dark either way.
 */
enum class AppTheme(val key: String) {
    SYSTEM("system"), LIGHT("light"), DARK("dark");

    companion object {
        fun fromKey(k: String?): AppTheme = entries.firstOrNull { it.key == k } ?: DARK
    }
}

object DictionaryMerge {
    /** Import semantics: an incoming row replaces any existing row with the same word (case-insensitive). */
    fun merge(existing: List<DictEntry>, incoming: List<DictEntry>): List<DictEntry> {
        val inc = LinkedHashMap<String, DictEntry>()
        for (e in incoming) { val k = e.word.lowercase(); inc.remove(k); inc[k] = e }
        return existing.filter { it.word.lowercase() !in inc } + inc.values
    }
}

data class Settings(
    /**
     * Dictation language: a whisper code, [LANGUAGE_AUTO] = "Automatic" (detect; the default
     * since D113), [LANGUAGE_KEYBOARD] = the active keyboard's language at dictation start (the
     * default 2026-10-31 to D113), or [LANGUAGE_UNSET] = the phone's language when the loaded
     * whisper model is multilingual and knows it, else auto ([LanguageDefault]).
     */
    val language: String = LANGUAGE_AUTO,
    val cleanup: CleanupLevel = CleanupLevel.NONE,
    val retention: Retention = Retention.KEEP,
    /** D35: how many of the newest History rows keep their recording for "Retry with current settings" (0 = none). */
    val retryAudioKeep: Int = RetryLogic.DEFAULT_KEEP,
    val capMinutes: Int = 5,
    val mockEngine: Boolean = false,
    val userDenylist: Set<String> = emptySet(),
    /** Local LLM cleanup (murmur-llm plugin + an imported GGUF). Off by default. */
    val llmEnabled: Boolean = false,
    /** Live preview of partial transcripts next to the bubble. Off by default. */
    val livePreview: Boolean = DEFAULT_LIVE_PREVIEW,
    /** Shake to bring a snoozed bubble back / reveal it for 10 s. */
    val shakeToReveal: Boolean = true,
    /** Shrink the Ready bubble to 36dp after 5 s. */
    val shrinkBubble: Boolean = true,
    /** "Shrink to a dot": the shrunk bubble is drawn as a plain 16dp dot (touch target stays 36dp); only used while [shrinkBubble]. */
    val shrinkToDot: Boolean = true,
    /** "Bubble size": multiplies every bubble dimension; 0.75..1.5 on a 5 % grid ([bubbleScaleOf]). */
    val bubbleScale: Float = 1f,
    /** Command Mode's ✎ button next to the mic: off, while text is selected (default), or always (also speak-to-generate). */
    val commandButton: CommandButton = CommandButton.WITH_SELECTION,
    /**
     * D90: the Translate button (⇄) next to the mic: dictate, and it is typed translated.
     * null = automatic: shown while two or more keyboard languages are enabled ([showsTranslate]).
     */
    val translateButton: Boolean? = null,
    /** D90: the language the Translate button types in (whisper code); "" = the other keyboard language. */
    val translateTo: String = "",
    /** core-p1 deterministic spoken cleanup ("Auto-format punctuation, fillers and corrections"). */
    val spokenCleanup: Boolean = true,
    /** On-device ASR engine; Parakeet is English-only (D4/D12). */
    val asrEngine: dev.murmur.core.AsrEngine = dev.murmur.core.AsrEngine.WHISPER,
    /**
     * D19: a hands-free (tap) dictation ends after this much trailing silence; 0 = only when tapped
     * again, the default since D104 (it was 1.5 s; an install from before keeps that, [EndpointDefault]).
     */
    val endpointSilenceMs: Int = DEFAULT_ENDPOINT_SILENCE_MS,
    /** D27 "Writing style": per-bucket style; chats casual, email + other formal ([Styles] defaults). */
    val styles: dev.murmur.core.Styles = dev.murmur.core.Styles(),
    /** "Writing style → Apps": package → bucket, over the built-in [AppCategorizer] lists. */
    val appBuckets: Map<String, AppCategorizer.Category> = emptyMap(),
    /**
     * "Learn from my edits" (Windows `learn_from_edits`): after an insert, re-read that field for
     * up to 45 s and offer corrected words as Dictionary suggestions (never added without Add).
     */
    val learnFromEdits: Boolean = true,
    /**
     * D54 "Keep both languages loaded": with a Hebrew model and a default model installed, load
     * both at once so a keyboard-language switch never reloads (more RAM: ≈ 0.5 GB with whisper
     * base + Zipformer Hebrew, up to ≈ 1.1 GB with Parakeet). Off by default.
     */
    val keepBothLanguages: Boolean = false,
    /** Light / dark for the app's screens ([AppTheme]); dark by default (D104). */
    val theme: AppTheme = AppTheme.DARK,
) {
    /** D37 / D58: the dictation language follows the keyboard (read at the start and again at the stop). */
    val followsKeyboard: Boolean get() = language == LANGUAGE_KEYBOARD

    companion object {
        /**
         * D113 "Automatic", the default: the speech decides (whisper / the server's language ID);
         * an installed language model is also used when the keyboard is in that language
         * ([LanguageDefault.resolve]), since the phone has no spoken language ID (D34).
         */
        const val LANGUAGE_AUTO = "auto"

        /** "Phone language": follow the phone ([LanguageDefault.effective]). */
        const val LANGUAGE_UNSET = ""

        /** D37 "Keyboard language": the active keyboard's language (D58: the one active when the recording stops), else the phone's. */
        const val LANGUAGE_KEYBOARD = "keyboard"

        /** [SessionConfig.languages] for an effective [language]: unset / "auto" = detect (empty). */
        fun sessionLanguages(language: String): List<String> =
            if (language == "auto" || language.isBlank()) emptyList() else listOf(language)

        /**
         * The stored language after the 2026-10-23 default change (auto → phone language).
         * Settings are saved as a whole and "auto" was the first, default entry, so a stored
         * "auto" from before the change is almost always untouched: it becomes unset once
         * ([migrated] false). Anything else, and "auto" saved after the change, is kept.
         */
        fun migrateLanguage(stored: String?, migrated: Boolean): String = when {
            stored == null -> LANGUAGE_UNSET
            !migrated && stored == "auto" -> LANGUAGE_UNSET
            else -> stored
        }

        /**
         * The stored language after every default change, once each ([migrated] /
         * [keyboardMigrated] false = that change has not run on this store). Nothing stored (a
         * new install) is [LANGUAGE_AUTO] (D113). "Phone language" ([LANGUAGE_UNSET], the untouched
         * default until 2026-10-31, after [migrateLanguage]) that the D37 move never reached
         * becomes [LANGUAGE_AUTO] too (D37 moved it to Keyboard language; D113 made Automatic the
         * default instead). D113, once ([autoMigrated] false): a stored Keyboard language (the
         * default since D37, so almost always untouched) becomes [LANGUAGE_AUTO]; Keyboard
         * language picked after that move is kept. An explicit language, Automatic and Phone
         * language are always kept.
         */
        fun migrateDefaultLanguage(stored: String?, migrated: Boolean, keyboardMigrated: Boolean, autoMigrated: Boolean): String {
            if (stored == null) return LANGUAGE_AUTO
            val v = migrateLanguage(stored, migrated)
            return when {
                !keyboardMigrated && v == LANGUAGE_UNSET -> LANGUAGE_AUTO
                !autoMigrated && v == LANGUAGE_KEYBOARD -> LANGUAGE_AUTO
                else -> v
            }
        }

        /** "Bubble size" slider: 75 %..150 % in 5 % steps, 100 % = the spec sizes. */
        val BUBBLE_SIZE_PERCENT = 75..150
        const val BUBBLE_SIZE_STEP_PERCENT = 5

        /** A stored or picked size, on the 5 % grid and inside the range (bad input = 100 %). */
        fun bubbleScaleOf(v: Float): Float {
            if (!v.isFinite()) return 1f
            val pct = kotlin.math.round(v * 100f / BUBBLE_SIZE_STEP_PERCENT).toInt() * BUBBLE_SIZE_STEP_PERCENT
            return pct.coerceIn(BUBBLE_SIZE_PERCENT) / 100f
        }

        /**
         * The AI cleanup strength when none was picked (stored NONE: the picker offers only
         * Light / Medium / High). Medium; it was Light. A picked strength is kept, and an
         * install from before keeps the Light it ran at ([StrengthDefault]).
         */
        val DEFAULT_STRENGTH = CleanupLevel.MEDIUM

        /** The strength a [stored] level runs at when AI cleanup is on. */
        fun strengthOf(stored: CleanupLevel) = stored.takeIf { it != CleanupLevel.NONE } ?: DEFAULT_STRENGTH

        /** Phones decode slower than the desktop default budget of 1.5 s. */
        const val ANDROID_LLM_TIMEOUT_MS = 3_000L
        const val PARTIAL_INTERVAL_MS = 1_500
        /**
         * Live preview starts off: partial decodes cost battery and can delay the commit
         * (docs/proposals/streaming-asr-latency.md). A stored choice is kept as is.
         */
        const val DEFAULT_LIVE_PREVIEW = false
        /** D104: a tap dictation ends only when tapped again, unless a silence is set. */
        const val DEFAULT_ENDPOINT_SILENCE_MS = 0
        /**
         * D22: `murmur_core::endpoint::RECOMMENDED_SILENCE_MS` (was 2000 until 2026-10-23); the
         * default until D104, which installs from before keep.
         */
        const val RECOMMENDED_ENDPOINT_SILENCE_MS = 1_500
        const val OLD_DEFAULT_ENDPOINT_SILENCE_MS = 2_000
        val ENDPOINT_RANGE_MS = 300..10_000
        /**
         * D22 live transcription, always on since D114: every dictation sends this
         * (`murmur_core::segment::RECOMMENDED_PAUSE_MS`). The core ignores it for an engine that
         * must not see audio before commit (the cloud route, D18); an older server ignores it too.
         */
        const val SEGMENT_PAUSE_MS = 800

        /**
         * The stored hands-free endpoint after the D22 default change. Settings are saved as a
         * whole, so a stored 2000 from before the change is almost always the untouched old
         * default: it becomes 1500 once ([migrated] false). Any other value, and anything saved
         * after the migration ran, is the user's and is kept. Nothing stored is a fresh install
         * (D104: [EndpointDefault] writes 1500 down for an install from before), so the default.
         */
        fun migrateEndpointMs(stored: Int?, migrated: Boolean): Int = when {
            stored == null -> DEFAULT_ENDPOINT_SILENCE_MS
            !migrated && stored == OLD_DEFAULT_ENDPOINT_SILENCE_MS -> RECOMMENDED_ENDPOINT_SILENCE_MS
            else -> stored
        }

        /** The Settings field: blank / 0 = off; anything else is clamped to [ENDPOINT_RANGE_MS]. */
        fun parseEndpointMs(text: String): Int {
            val v = text.trim().toIntOrNull() ?: return 0
            return if (v <= 0) 0 else v.coerceIn(ENDPOINT_RANGE_MS)
        }
    }

    /** `SessionConfig.endpoint_silence_ms` for a hands-free session; null = off. */
    fun handsFreeEndpointMs(): Int? = endpointSilenceMs.takeIf { it > 0 }?.coerceIn(ENDPOINT_RANGE_MS)

    /**
     * The effective cleanup level: LLM off → NONE; on with NONE selected → [strength]. Since D27 NONE
     * (and `raw_mode`) only switch the LLM off: the deterministic formatting still runs whenever
     * [spokenCleanup] ("Auto-format") is on.
     */
    val effectiveCleanup: CleanupLevel
        get() = if (!llmEnabled) CleanupLevel.NONE else strength

    /** The strength AI cleanup runs at when it is on: the picked one, else [DEFAULT_STRENGTH]. */
    val strength: CleanupLevel
        get() = strengthOf(cleanup)

    /**
     * D28: with cloud cleanup on (OpenRouter, consent given) the LLM step runs even when the
     * on-device AI is off, at the chosen strength ([strength]). D36: likewise
     * with your server's cleanup on ([DevSettings.offPhoneCleanup]).
     */
    fun cleanupFor(cloudCleanup: Boolean): CleanupLevel =
        if (cloudCleanup && effectiveCleanup == CleanupLevel.NONE) strength
        else effectiveCleanup

    /** [language]: the effective one ([LanguageDefault.effective]); unset / "auto" = detect. [cloudCleanup]: see [cleanupFor]. */
    fun toSessionConfig(dictionary: List<DictEntry>, language: String = this.language, cloudCleanup: Boolean = false) = SessionConfig(
        languages = sessionLanguages(language),
        dictionary = dictionary,
        cleanupLevel = cleanupFor(cloudCleanup),
        rawMode = cleanupFor(cloudCleanup) == CleanupLevel.NONE,
        maxSamples = sessionCap().maxSamples,
        partialIntervalMs = if (livePreview) PARTIAL_INTERVAL_MS else null,
        llmTimeoutMs = ANDROID_LLM_TIMEOUT_MS,
        spokenCleanup = spokenCleanup,
        segmentPauseMs = SEGMENT_PAUSE_MS,
        styles = styles,
    )

    /** [ctx] with the user's bucket for its app ([appBuckets]); unchanged when there is none. */
    fun withAppBucket(ctx: FieldContext): FieldContext =
        appBuckets[ctx.appId]?.let { ctx.copy(appCategory = it.coreName) } ?: ctx

    fun sessionCap() = SessionCap(capMinutes.coerceIn(1, 20) * 60_000L)
}

/**
 * The dictation language when the user has not chosen one (2026-10-23: with `auto`, a 2 s English
 * clip on multilingual base came out as Bulgarian; whisper's language ID is unreliable on short,
 * accented phone speech). Unset = the phone's language if the loaded whisper model is
 * multilingual and whisper knows that language, else "auto". D37: [Settings.LANGUAGE_KEYBOARD]
 * applies the same rule to the active keyboard's language ([KeyboardLanguage]) and falls back to
 * the phone's when the keyboard's is unknown. Explicit choices are kept.
 */
object LanguageDefault {
    /** whisper.cpp's language codes (g_lang). */
    val WHISPER_LANGUAGES: Set<String> = (
        "en zh de es ru ko fr ja pt tr pl ca nl ar sv it id hi fi vi he uk el ms cs ro da hu ta no th ur hr bg lt la " +
            "mi ml cy sk te fa lv bn sr az sl kn et mk br eu is hy ne mn bs kk sq sw gl mr pa si km sn yo so af oc ka be " +
            "tg sd gu am yi lo uz fo ht ps tk nn mt sa lb my bo tl mg as tt haw ln ha ba jw su yue"
        ).split(' ').toSet()

    /** java.util.Locale's legacy codes and whisper's own Javanese code. */
    private val ALIASES = mapOf("iw" to "he", "in" to "id", "ji" to "yi", "jv" to "jw", "nb" to "no")

    /** "iw" → "he", "jv" → "jw", …; lowercase; null for null. */
    fun normalize(code: String?): String? = code?.lowercase()?.let { ALIASES[it] ?: it }

    /** English-only whisper files: `*.en.bin`, `*.en-q5_1.bin`, distil-whisper. Parakeet: English only. */
    fun isMultilingual(asr: LoadedAsr): Boolean {
        if (asr.engine != dev.murmur.core.AsrEngine.WHISPER) return false
        val n = asr.model.lowercase()
        return !Regex("""\.en([._-]|$)""").containsMatchIn(n) && "distil" !in n
    }

    /** Where the dictation language came from (debug log `language_source`). */
    enum class Source(val wire: String) { KEYBOARD("keyboard"), PHONE("phone"), FIXED("fixed") }

    /** The language the next dictation uses ("auto" = detect) and where it came from. */
    data class Resolved(val language: String, val source: Source) {
        /** [SessionConfig.languages] for it ([Settings.sessionLanguages]). */
        val sessionLanguages: List<String> get() = Settings.sessionLanguages(language)
    }

    /**
     * [deviceLanguage] = `Locale.getDefault().language`; [asr] = the loaded local ASR (null: none);
     * [installed] = languages with their own imported model (H5: `he` with a Hebrew model), which
     * the phone (or keyboard) language selects whatever is loaded right now; [keyboard] = the
     * active keyboard's language ([KeyboardLanguage.of]; null = unknown); [offPhone] = the
     * dictation goes to the server or the cloud ([DevSettings.expectedRoute]), which detect the
     * language themselves, so Automatic sends none there (D113).
     */
    fun resolve(
        setting: String, deviceLanguage: String?, asr: LoadedAsr?, installed: Set<String> = emptySet(), keyboard: String? = null,
        offPhone: Boolean = false,
    ): Resolved {
        fun follow(lang: String?): String {
            val l = normalize(lang)
            if (l != null && l in installed) return l
            return if (asr != null && isMultilingual(asr) && l != null && l in WHISPER_LANGUAGES) l else "auto"
        }
        val kb = normalize(keyboard)?.takeIf { it in WHISPER_LANGUAGES }
        return when (setting) {
            Settings.LANGUAGE_KEYBOARD ->
                if (kb != null) Resolved(follow(kb), Source.KEYBOARD) else Resolved(follow(deviceLanguage), Source.PHONE)
            Settings.LANGUAGE_UNSET -> Resolved(follow(deviceLanguage), Source.PHONE)
            // D113: off the phone the server's (or cloud's) language ID decides. On the phone,
            // which has none, a keyboard in a language with its own model (Hebrew) picks that
            // model; anything else is detected from the speech.
            Settings.LANGUAGE_AUTO ->
                kb?.takeIf { !offPhone && it in installed }?.let { Resolved(it, Source.KEYBOARD) } ?: Resolved(setting, Source.FIXED)
            else -> Resolved(setting, Source.FIXED)
        }
    }

    fun effective(setting: String, deviceLanguage: String?, asr: LoadedAsr?, installed: Set<String> = emptySet(), keyboard: String? = null): String =
        resolve(setting, deviceLanguage, asr, installed, keyboard).language

    /**
     * D58 / D113: the keyboard is read again when the recording stops (a switch re-runs the
     * recording in the stop language): always for Keyboard language; for Automatic only on the
     * phone ([offPhone] false) with a language model [installed], the one case the keyboard decides.
     */
    fun rereadsAtStop(setting: String, installed: Set<String>, offPhone: Boolean): Boolean = when (setting) {
        Settings.LANGUAGE_KEYBOARD -> true
        Settings.LANGUAGE_AUTO -> !offPhone && installed.isNotEmpty()
        else -> false
    }

    fun name(code: String): String = when (code) {
        Settings.LANGUAGE_AUTO -> "Automatic"
        "jw" -> "Javanese"
        "yue" -> "Cantonese"
        else -> java.util.Locale(code).getDisplayLanguage(java.util.Locale.ENGLISH).ifEmpty { code }
    }

    /** Main screen: what the next dictation uses, and why. */
    fun label(setting: String, effective: String, source: Source? = null): String = when {
        setting == Settings.LANGUAGE_KEYBOARD && source == Source.KEYBOARD ->
            if (effective == "auto") "Auto-detect (keyboard language not used: model is English-only or unknown)"
            else "${name(effective)} (keyboard language)"
        setting == Settings.LANGUAGE_KEYBOARD ->
            if (effective == "auto") "Auto-detect (keyboard language unknown; phone language not used)"
            else "${name(effective)} (phone language; keyboard language unknown)"
        setting == Settings.LANGUAGE_AUTO && effective != Settings.LANGUAGE_AUTO -> "${name(effective)} (Automatic: ${name(effective)} keyboard and model)"
        setting != Settings.LANGUAGE_UNSET -> name(effective)
        effective == "auto" -> "Auto-detect (phone language not used: model is English-only or unknown)"
        else -> "${name(effective)} (phone language)"
    }
}

/**
 * D37: the active keyboard's language, from `InputMethodManager.getCurrentInputMethodSubtype()`:
 * its `languageTag` ("he-IL"), else the deprecated `locale` ("iw_IL") when the tag is empty.
 * Returns whisper's code ("he"), or null when there is none or whisper does not know it (emoji /
 * handwriting subtypes, "und", "zz"): the caller then falls back to the phone language.
 */
object KeyboardLanguage {
    fun of(languageTag: String?, locale: String?): String? {
        val raw = languageTag?.trim()?.takeIf { it.isNotEmpty() } ?: locale?.trim()?.takeIf { it.isNotEmpty() } ?: return null
        val primary = raw.split('-', '_').first()
        return LanguageDefault.normalize(primary)?.takeIf { it in LanguageDefault.WHISPER_LANGUAGES }
    }
}

/**
 * captureFieldSnapshot → core `FieldContext`: ±500 chars around the selection
 * from `getSurroundingText(500, 500, 0)`; blank for context-extraction-excluded fields
 * (sensitive / URI inputType) and banking apps.
 */
object FieldContextBuilder {
    const val WINDOW = 500

    fun build(pkg: String?, inputType: Int, surrounding: String?, selStart: Int, selEnd: Int, browserHost: String? = null): FieldContext {
        val category = AppCategorizer.categorize(pkg, browserHost).coreName
        val base = FieldContext(appId = pkg.orEmpty(), urlHost = browserHost.orEmpty(), appCategory = category)
        if (surrounding == null || EditorInputType.isContentExtractionExcluded(inputType) || BubbleVisibilityPolicy.isRestricted(pkg)) return base
        val len = surrounding.length
        val a = minOf(selStart, selEnd).coerceIn(0, len)
        val b = maxOf(selStart, selEnd).coerceIn(0, len)
        return base.copy(
            before = surrounding.substring(0, a).takeLast(WINDOW),
            selected = surrounding.substring(a, b),
            after = surrounding.substring(b).take(WINDOW),
        )
    }
}

/** When the bubble shows Command Mode's ✎ button (Settings → Bubble). */
enum class CommandButton(val key: String) {
    OFF("off"),
    /** Only while the focused field has a selection: tap ✎, say "make it shorter". */
    WITH_SELECTION("selection"),
    /** Whenever the bubble is up; without a selection the instruction writes new text at the cursor. */
    ALWAYS("always");

    /** Show ✎ now? [hasSelection] = the focused field has a non-empty selection. */
    fun shows(hasSelection: Boolean): Boolean = this == ALWAYS || (this == WITH_SELECTION && hasSelection)

    companion object {
        fun fromKey(k: String?): CommandButton = entries.firstOrNull { it.key == k } ?: WITH_SELECTION
    }
}

/** D90: the ⇄ shows when the setting says so; unset, when [keyboardLanguages] offers something to translate into. */
fun Settings.showsTranslate(keyboardLanguages: Int): Boolean = translateButton ?: (keyboardLanguages >= 2)
