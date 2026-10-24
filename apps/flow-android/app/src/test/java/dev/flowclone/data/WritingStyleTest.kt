package dev.flowclone.data

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.FieldContext
import dev.flowclone.core.Style
import dev.flowclone.core.Styles
import dev.flowclone.policy.AppCategorizer.Category
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D27 layer 1 (docs/findings/2026-10-24-formatting-gap.md §7): style defaults, per-app buckets, labels. */
class WritingStyleTest {
    @Test fun chatBucketsDefaultToCasualEmailAndOtherStayFormal() {
        val s = Settings().styles
        assertEquals(Style.CASUAL, s.personal)
        assertEquals(Style.CASUAL, s.work)
        assertEquals(Style.FORMAL, s.email)
        assertEquals(Style.FORMAL, s.other)
        assertEquals("the core-facing default matches", s, Styles())
    }

    @Test fun sessionConfigSendsTheStyles() {
        val styles = Styles(personal = Style.VERY_CASUAL, work = Style.FORMAL, email = Style.CASUAL, other = Style.FORMAL)
        assertEquals(styles, Settings(styles = styles).toSessionConfig(emptyList()).styles)
        assertEquals(Styles(), Settings().toSessionConfig(emptyList()).styles)
    }

    @Test fun styleForEachCoreCategory() {
        val s = Styles(personal = Style.VERY_CASUAL, work = Style.CASUAL, email = Style.FORMAL, other = Style.EXCITED)
        assertEquals(Style.VERY_CASUAL, s.forCategory("personal_chat"))
        assertEquals(Style.CASUAL, s.forCategory("work_chat"))
        assertEquals(Style.FORMAL, s.forCategory("email"))
        for (c in listOf("other", "document", "ai_prompt", "terminal", "")) assertEquals(c, Style.EXCITED, s.forCategory(c))
    }

    @Test fun styleWireRoundTrips() {
        for (st in Style.entries) assertEquals(st, Style.fromWire(st.wire))
        assertNull(Style.fromWire("shouty"))
        assertNull(Style.fromWire(null))
    }

    @Test fun perAppBucketOverridesTheBuiltInList() {
        val s = Settings(appBuckets = mapOf("com.example.chat" to Category.PERSONAL, "com.Slack" to Category.EMAIL))
        assertEquals("personal_chat", s.withAppBucket(FieldContext(appId = "com.example.chat", appCategory = "other")).appCategory)
        assertEquals("email", s.withAppBucket(FieldContext(appId = "com.Slack", appCategory = "work_chat")).appCategory)
        val untouched = FieldContext(appId = "com.whatsapp", appCategory = "personal_chat", before = "x")
        assertEquals(untouched, s.withAppBucket(untouched))
    }

    @Test fun overridesParseFormatAndReject() {
        val ok = WritingStyle.parseOverrides(" com.example.chat = personal\n\norg.foo_bar.Baz=Work\ncom.x.mail = email \ncom.notes.app=other")
        assertEquals(
            mapOf(
                "com.example.chat" to Category.PERSONAL, "org.foo_bar.Baz" to Category.WORK,
                "com.x.mail" to Category.EMAIL, "com.notes.app" to Category.OTHER,
            ),
            ok.map,
        )
        assertTrue(ok.errors.isEmpty())
        assertEquals("com.a.b = work\ncom.z.y = personal", WritingStyle.formatOverrides(mapOf("com.z.y" to Category.PERSONAL, "com.a.b" to Category.WORK)))
        assertEquals(ok.map, WritingStyle.parseOverrides(WritingStyle.formatOverrides(ok.map)).map)
        val bad = WritingStyle.parseOverrides("com.ok.app = work\nnot a package = work\ncom.x.y = loud\ncom.q.r")
        assertEquals(listOf("not a package = work", "com.x.y = loud", "com.q.r"), bad.errors)
        assertTrue("last line for a package wins", WritingStyle.parseOverrides("com.a.b=work\ncom.a.b=email").map == mapOf("com.a.b" to Category.EMAIL))
    }

    @Test fun hubSubtitle() {
        assertEquals("Chats casual · Email formal", WritingStyle.summary(Settings()))
        assertEquals(
            "Personal very casual · Work formal · Email formal",
            WritingStyle.summary(Settings(styles = Styles(personal = Style.VERY_CASUAL, work = Style.FORMAL))),
        )
        assertEquals(
            "Chats casual · Email formal · Other casual",
            WritingStyle.summary(Settings(styles = Styles(other = Style.CASUAL))),
        )
        assertEquals(
            "Chats casual · Email formal · 2 app overrides",
            WritingStyle.summary(Settings(appBuckets = mapOf("com.a.b" to Category.WORK, "com.c.d" to Category.EMAIL))),
        )
    }

    @Test fun everyBucketOffersFormalCasualVeryCasualWithAPreview() {
        assertEquals(listOf(Style.FORMAL, Style.CASUAL, Style.VERY_CASUAL), WritingStyle.CHOICES.map { it.style })
        for (c in WritingStyle.CHOICES) assertTrue(c.name, c.preview.isNotBlank())
        assertEquals(listOf("Personal messages", "Work messages", "Email", "Other"), WritingStyle.BUCKETS.map { it.title })
        // A style the page does not offer (EXCITED, set elsewhere) is still shown as picked, not lost.
        assertEquals(listOf(Style.FORMAL, Style.CASUAL, Style.VERY_CASUAL, Style.EXCITED), WritingStyle.choicesFor(Style.EXCITED).map { it.style })
    }

    @Test fun bucketGetAndSet() {
        val b = WritingStyle.BUCKETS.first { it.category == Category.WORK }
        val s = b.set(Styles(), Style.VERY_CASUAL)
        assertEquals(Style.VERY_CASUAL, s.work)
        assertEquals(Style.VERY_CASUAL, b.get(s))
        assertEquals(Style.CASUAL, s.personal)
    }

    // ---- the LLM part of a result: llm_skipped_clean is a normal outcome, not a failure ----

    @Test fun llmOutcomeCodesAndLabels() {
        fun r(st: CommitStatus, fb: String?) = CommitResult(status = st, fallbackReason = fb)
        assertEquals("formatted", LlmOutcome.code(r(CommitStatus.FORMATTED, null)))
        assertEquals("llm_skipped_clean", LlmOutcome.code(r(CommitStatus.RAW_TRANSCRIPT, "llm_skipped_clean")))
        assertNull("LLM off", LlmOutcome.code(r(CommitStatus.RAW_TRANSCRIPT, null)))
        assertNull(LlmOutcome.code(null))
        assertEquals("AI cleaned", LlmOutcome.label("formatted"))
        assertEquals("AI not needed (already clean)", LlmOutcome.label("llm_skipped_clean"))
        assertEquals("AI too slow, formatted text used", LlmOutcome.label("llm_timeout"))
        assertEquals("AI result not used (llm_dropped), formatted text used", LlmOutcome.label("llm_dropped"))
        assertNull(LlmOutcome.label(null))
        assertTrue(LlmOutcome.isNormal("llm_skipped_clean"))
        assertTrue(LlmOutcome.isNormal("formatted"))
        assertTrue(LlmOutcome.isNormal(null))
        assertTrue(!LlmOutcome.isNormal("llm_timeout"))
    }
}
