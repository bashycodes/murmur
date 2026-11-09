package dev.murmur.data

import dev.murmur.core.CommitResult
import dev.murmur.core.CommitStatus
import dev.murmur.core.Style
import dev.murmur.core.Styles
import dev.murmur.policy.AppCategorizer.Category

/**
 * Settings → Writing style (D27 layer 1, docs/findings/2026-10-24-formatting-gap.md §7): one style
 * per bucket, picked by the app the dictation goes into ([dev.murmur.policy.AppCategorizer],
 * plus the user's per-app [Settings.appBuckets]).
 *
 * What a style does in murmur-core: FORMAL keeps every period; CASUAL drops the final period (≤ 10
 * sentences, cursor context permitting); VERY_CASUAL always drops it, and with AI cleanup on the
 * model also writes all lowercase. EXCITED ("!") exists in the core but is not offered here.
 */
object WritingStyle {
    class Bucket(val category: Category, val title: String, val examples: String, val get: (Styles) -> Style, val set: (Styles, Style) -> Styles)

    val BUCKETS = listOf(
        Bucket(Category.PERSONAL, "Personal messages", "WhatsApp, Messages, Telegram, Signal, Messenger, Discord, Instagram", { it.personal }, { s, v -> s.copy(personal = v) }),
        Bucket(Category.WORK, "Work messages", "Slack, Teams, Google Chat", { it.work }, { s, v -> s.copy(work = v) }),
        Bucket(Category.EMAIL, "Email", "Gmail, Outlook", { it.email }, { s, v -> s.copy(email = v) }),
        Bucket(Category.OTHER, "Other", "Notes, documents, browsers, everything else", { it.other }, { s, v -> s.copy(other = v) }),
    )

    /** [summary]: the Style tab card's short rule (AndroidStyle.dc.html "Capitals, full punctuation"). */
    class Choice(val style: Style, val name: String, val preview: String, val rule: String, val summary: String) {
        /** The choice row's description: the example line, then what changes. */
        val description: String get() = "“$preview” · $rule"

        /** What TalkBack reads for the Style tab card: name, short rule, example. */
        val cardDescription: String get() = "$name. $summary. Example: $preview"
    }

    val CHOICES = listOf(
        Choice(Style.FORMAL, "Formal", "Sounds good. See you at 7:30.", "Capitals and full punctuation", "Capitals, full punctuation"),
        Choice(Style.CASUAL, "Casual", "Sounds good. See you at 7:30", "Capitals, no final period", "Capitals, lighter punctuation"),
        Choice(Style.VERY_CASUAL, "Very casual", "sounds good. see you at 7:30", "No final period; all lowercase when AI cleanup is on", "Lowercase, lighter punctuation"),
    )

    private val EXCITED = Choice(Style.EXCITED, "Excited", "Sounds good! See you at 7:30!", "More exclamation marks (AI cleanup)", "More exclamation marks")

    /** The bucket of [c] (Style tab: one segment per bucket). */
    fun bucket(c: Category): Bucket = BUCKETS.first { it.category == c }

    /** [CHOICES], plus [current] when it is a core style this page does not offer (kept, not lost). */
    fun choicesFor(current: Style): List<Choice> = if (CHOICES.any { it.style == current }) CHOICES else CHOICES + EXCITED

    fun name(s: Style): String = (CHOICES + EXCITED).first { it.style == s }.name

    /** Hub row subtitle: "Chats casual · Email formal" (+ Other when it differs from Email, + overrides). */
    fun summary(s: Settings): String {
        val st = s.styles
        fun n(x: Style) = name(x).lowercase()
        val parts = if (st.personal == st.work) mutableListOf("Chats ${n(st.personal)}")
        else mutableListOf("Personal ${n(st.personal)}", "Work ${n(st.work)}")
        parts += "Email ${n(st.email)}"
        if (st.other != st.email) parts += "Other ${n(st.other)}"
        when (val k = s.appBuckets.size) {
            0 -> Unit
            1 -> parts += "1 app override"
            else -> parts += "$k app overrides"
        }
        return parts.joinToString(" · ")
    }

    // ---- per-app overrides: "com.example.app = personal|work|email|other", one per line ----

    private val PACKAGE = Regex("""[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+""")
    private val KEYS = mapOf("personal" to Category.PERSONAL, "work" to Category.WORK, "email" to Category.EMAIL, "other" to Category.OTHER)

    fun key(c: Category): String = KEYS.entries.first { it.value == c }.key

    /** [errors] = the offending lines as typed (trimmed); [map] holds the valid lines (last one per package wins). */
    data class Parsed(val map: Map<String, Category>, val errors: List<String>)

    fun parseOverrides(text: String): Parsed {
        val map = LinkedHashMap<String, Category>()
        val errors = ArrayList<String>()
        for (raw in text.lines()) {
            val line = raw.trim()
            if (line.isEmpty()) continue
            val pkg = line.substringBefore('=', "").trim()
            val bucket = KEYS[line.substringAfter('=', "").trim().lowercase()]
            if (!PACKAGE.matches(pkg) || bucket == null) errors += line else { map.remove(pkg); map[pkg] = bucket }
        }
        return Parsed(map, errors)
    }

    fun formatOverrides(m: Map<String, Category>): String =
        m.entries.sortedBy { it.key }.joinToString("\n") { "${it.key} = ${key(it.value)}" }
}

/**
 * The LLM part of a dictation result, for the Developer "Active engine" row, history and the
 * debug log (`llm` field). `llm_skipped_clean` (D27: nothing for the model to do) is a normal
 * outcome, like "formatted": the deterministic text was already the answer.
 */
object LlmOutcome {
    const val FORMATTED = "formatted"
    const val SKIPPED_CLEAN = "llm_skipped_clean"

    /** "formatted" when the model's text was used, else the core's `fallback_reason`; null = the LLM did not run (off). */
    fun code(r: CommitResult?): String? = when {
        r == null -> null
        r.status == CommitStatus.FORMATTED -> FORMATTED
        else -> r.fallbackReason
    }

    fun label(code: String?): String? = when (code) {
        null -> null
        FORMATTED -> "AI cleaned"
        SKIPPED_CLEAN -> "AI not needed (already clean)"
        "llm_timeout" -> "AI too slow, formatted text used"
        // D28: refused up front (cloud cleanup: no consent / key / budget, open circuit, no private route).
        else -> if (code.startsWith("llm_error:")) code.removePrefix("llm_error:").trim().let { c ->
            "AI unavailable (${CloudText.hint(c) ?: c}), formatted text used"
        } else "AI result not used ($code), formatted text used"
    }

    fun isNormal(code: String?): Boolean = code == null || code == FORMATTED || code == SKIPPED_CLEAN

    /** [offReason]: no cleanup engine is switched on (phone AI off, no OpenRouter or server cleanup). */
    const val OFF_ALL = "ai_cleanup_off"
    /** [offReason]: the phone AI is on but no model is imported. */
    const val OFF_NO_MODEL = "no_phone_model"
    /** [offReason]: the phone AI is on but its model failed to load. */
    const val OFF_FAILED = "phone_model_failed"
    /** [offReason]: an engine is on but none could run (terminal app, snippet, server without an LLM, …). */
    const val OFF_NONE_AVAILABLE = "no_engine_available"

    /**
     * Debug line `llm_off` (2026-11-02, local-cleanup-skipped): why a dictation got no AI cleanup
     * at all (its `llm` is null), which used to be a silent gap. [phoneOn]: the phone's cleanup
     * switch ([CleanupEngines.on] LOCAL); [phone]: the engine's on-device LLM status (null =
     * unknown); [offPhone]: OpenRouter or server cleanup is on ([DevSettings.offPhoneCleanup]).
     */
    fun offReason(phoneOn: Boolean, phone: EnginePlan.LlmStatus?, offPhone: Boolean): String = when {
        offPhone -> OFF_NONE_AVAILABLE
        !phoneOn -> OFF_ALL
        phone is EnginePlan.LlmStatus.NoModel -> OFF_NO_MODEL
        phone is EnginePlan.LlmStatus.Failed -> OFF_FAILED
        else -> OFF_NONE_AVAILABLE
    }

    fun offLabel(reason: String?): String? = when (reason) {
        null -> null
        OFF_ALL -> "no AI cleanup (all cleanup engines off)"
        OFF_NO_MODEL -> "no AI cleanup (phone AI on, no model imported)"
        OFF_FAILED -> "no AI cleanup (phone model failed to load)"
        else -> "no AI cleanup ($reason)"
    }
}
