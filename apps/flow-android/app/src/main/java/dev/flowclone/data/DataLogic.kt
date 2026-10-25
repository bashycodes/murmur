package dev.flowclone.data

import dev.flowclone.core.CleanupLevel
import dev.flowclone.core.DictEntry
import dev.flowclone.core.FieldContext
import dev.flowclone.core.SessionConfig
import dev.flowclone.policy.AppCategorizer
import dev.flowclone.policy.BubbleVisibilityPolicy
import dev.flowclone.policy.EditorInputType
import dev.flowclone.session.SessionCap

/** History retention pref (10-clone-spec-android §4.8: keep / 24 h / never). */
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
     * Dictation language: a whisper code, "auto", or [LANGUAGE_UNSET] = the phone's language when
     * the loaded whisper model is multilingual and knows it, else auto ([LanguageDefault]).
     */
    val language: String = LANGUAGE_UNSET,
    val cleanup: CleanupLevel = CleanupLevel.NONE,
    val retention: Retention = Retention.KEEP,
    val capMinutes: Int = 5,
    val mockEngine: Boolean = false,
    val userDenylist: Set<String> = emptySet(),
    /** Local LLM cleanup (flow-llm plugin + an imported GGUF). Off by default (10 §5). */
    val llmEnabled: Boolean = false,
    /** Live preview of partial transcripts next to the bubble. */
    val livePreview: Boolean = true,
    /** Shake to bring a snoozed bubble back / reveal it for 10 s (07 §5.5). */
    val shakeToReveal: Boolean = true,
    /** Shrink the Ready bubble to 36dp after 5 s (07 §5.3 `shrinkBubbleEnabled`). */
    val shrinkBubble: Boolean = true,
    /** "Bubble size": multiplies every bubble dimension; 0.75..1.5 on a 5 % grid ([bubbleScaleOf]). */
    val bubbleScale: Float = 1f,
    /** core-p1 deterministic spoken cleanup ("Auto-format punctuation, fillers and corrections"). */
    val spokenCleanup: Boolean = true,
    /** On-device ASR engine; Parakeet is English-only (D4/D12). */
    val asrEngine: dev.flowclone.core.AsrEngine = dev.flowclone.core.AsrEngine.WHISPER,
    /** D19: a hands-free (tap) dictation ends after this much trailing silence; 0 = only when tapped again. */
    val endpointSilenceMs: Int = DEFAULT_ENDPOINT_SILENCE_MS,
    /**
     * D22 "Decode while speaking": decode finished phrases during recording so release → text
     * only waits for the tail. Per device (not in the synced dictionary). Local and server routes;
     * the core opts the cloud route out by itself.
     */
    val decodeWhileSpeaking: Boolean = true,
    /** D27 "Writing style": per-bucket style; chats casual, email + other formal ([Styles] defaults). */
    val styles: dev.flowclone.core.Styles = dev.flowclone.core.Styles(),
    /** "Writing style → Apps": package → bucket, over the built-in [AppCategorizer] lists. */
    val appBuckets: Map<String, AppCategorizer.Category> = emptyMap(),
) {
    companion object {
        /** [language] not chosen: follow the phone ([LanguageDefault.effective]). */
        const val LANGUAGE_UNSET = ""

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

        /** "Bubble size" slider: 75 %..150 % in 5 % steps, 100 % = the spec sizes. */
        val BUBBLE_SIZE_PERCENT = 75..150
        const val BUBBLE_SIZE_STEP_PERCENT = 5

        /** A stored or picked size, on the 5 % grid and inside the range (bad input = 100 %). */
        fun bubbleScaleOf(v: Float): Float {
            if (!v.isFinite()) return 1f
            val pct = kotlin.math.round(v * 100f / BUBBLE_SIZE_STEP_PERCENT).toInt() * BUBBLE_SIZE_STEP_PERCENT
            return pct.coerceIn(BUBBLE_SIZE_PERCENT) / 100f
        }

        /** Phones decode slower than the desktop default budget of 1.5 s. */
        const val ANDROID_LLM_TIMEOUT_MS = 3_000L
        const val PARTIAL_INTERVAL_MS = 1_500
        /** D22: `flow_core::endpoint::RECOMMENDED_SILENCE_MS` (was 2000 until 2026-10-23). */
        const val DEFAULT_ENDPOINT_SILENCE_MS = 1_500
        const val OLD_DEFAULT_ENDPOINT_SILENCE_MS = 2_000
        val ENDPOINT_RANGE_MS = 300..10_000
        /** D22: `flow_core::segment::RECOMMENDED_PAUSE_MS`. */
        const val SEGMENT_PAUSE_MS = 800

        /**
         * The stored hands-free endpoint after the D22 default change. Settings are saved as a
         * whole, so a stored 2000 from before the change is almost always the untouched old
         * default: it becomes the new default once ([migrated] false). Any other value, and
         * anything saved after the migration ran, is the user's and is kept.
         */
        fun migrateEndpointMs(stored: Int?, migrated: Boolean): Int = when {
            stored == null -> DEFAULT_ENDPOINT_SILENCE_MS
            !migrated && stored == OLD_DEFAULT_ENDPOINT_SILENCE_MS -> DEFAULT_ENDPOINT_SILENCE_MS
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
     * The effective cleanup level: LLM off → NONE; on with NONE selected → LIGHT. Since D27 NONE
     * (and `raw_mode`) only switch the LLM off: the deterministic formatting still runs whenever
     * [spokenCleanup] ("Auto-format") is on.
     */
    val effectiveCleanup: CleanupLevel
        get() = if (!llmEnabled) CleanupLevel.NONE else cleanup.takeIf { it != CleanupLevel.NONE } ?: CleanupLevel.LIGHT

    /**
     * D28: with cloud cleanup on (OpenRouter, consent given) the LLM step runs even when the
     * on-device AI is off, at the chosen strength (LIGHT when none was picked).
     */
    fun cleanupFor(cloudCleanup: Boolean): CleanupLevel =
        if (cloudCleanup && effectiveCleanup == CleanupLevel.NONE) cleanup.takeIf { it != CleanupLevel.NONE } ?: CleanupLevel.LIGHT
        else effectiveCleanup

    /** [language]: the effective one ([LanguageDefault.effective]); unset / "auto" = detect. [cloudCleanup]: see [cleanupFor]. */
    fun toSessionConfig(dictionary: List<DictEntry>, language: String = this.language, cloudCleanup: Boolean = false) = SessionConfig(
        languages = if (language == "auto" || language.isBlank()) emptyList() else listOf(language),
        dictionary = dictionary,
        cleanupLevel = cleanupFor(cloudCleanup),
        rawMode = cleanupFor(cloudCleanup) == CleanupLevel.NONE,
        maxSamples = sessionCap().maxSamples,
        partialIntervalMs = if (livePreview) PARTIAL_INTERVAL_MS else null,
        llmTimeoutMs = ANDROID_LLM_TIMEOUT_MS,
        spokenCleanup = spokenCleanup,
        segmentPauseMs = if (decodeWhileSpeaking) SEGMENT_PAUSE_MS else null,
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
 * multilingual and whisper knows that language, else "auto". Explicit choices are kept.
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

    /** English-only whisper files: `*.en.bin`, `*.en-q5_1.bin`, distil-whisper. Parakeet: English only. */
    fun isMultilingual(asr: LoadedAsr): Boolean {
        if (asr.engine != dev.flowclone.core.AsrEngine.WHISPER) return false
        val n = asr.model.lowercase()
        return !Regex("""\.en([._-]|$)""").containsMatchIn(n) && "distil" !in n
    }

    /** [deviceLanguage] = `Locale.getDefault().language`; [asr] = the loaded local ASR (null: none). */
    fun effective(setting: String, deviceLanguage: String?, asr: LoadedAsr?): String {
        if (setting != Settings.LANGUAGE_UNSET) return setting
        val lang = deviceLanguage?.lowercase()?.let { ALIASES[it] ?: it }
        return if (asr != null && isMultilingual(asr) && lang != null && lang in WHISPER_LANGUAGES) lang else "auto"
    }

    fun name(code: String): String = when (code) {
        "auto" -> "Auto-detect"
        "jw" -> "Javanese"
        "yue" -> "Cantonese"
        else -> java.util.Locale(code).getDisplayLanguage(java.util.Locale.ENGLISH).ifEmpty { code }
    }

    /** Main screen: what the next dictation uses, and why. */
    fun label(setting: String, effective: String): String = when {
        setting != Settings.LANGUAGE_UNSET -> name(effective)
        effective == "auto" -> "Auto-detect (phone language not used: model is English-only or unknown)"
        else -> "${name(effective)} (phone language)"
    }
}

/**
 * captureFieldSnapshot → core `FieldContext` (06 §1): ±500 chars around the selection
 * from `getSurroundingText(500, 500, 0)`; blank for context-extraction-excluded fields
 * (sensitive / URI inputType) and banking apps.
 */
object FieldContextBuilder {
    const val WINDOW = 500

    fun build(pkg: String?, inputType: Int, surrounding: String?, selStart: Int, selEnd: Int, browserHost: String? = null): FieldContext {
        val category = AppCategorizer.categorize(pkg, browserHost).coreName
        val base = FieldContext(appId = pkg.orEmpty(), urlHost = browserHost.orEmpty(), appCategory = category)
        if (surrounding == null || EditorInputType.isContentExtractionExcluded(inputType) || BubbleVisibilityPolicy.isBanking(pkg)) return base
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
