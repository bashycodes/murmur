package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CleanupLevel
import dev.flowclone.data.SettingsPages.Field
import dev.flowclone.data.SettingsPages.Page
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The main Settings hub + sub-pages: flavour visibility, form drafts / save gating, immediate effects, row subtitles. */
class SettingsPagesTest {
    private val s = Settings()

    // ---- pages per flavour (docs/hybrid-architecture.md §1.4: remote hides model import / LLM) ----

    @Test fun remoteFlavourHidesOnDeviceEngineAndAiCleanup() {
        val remote = SettingsPages.visiblePages(BuildCaps.REMOTE)
        assertFalse(Page.ENGINE in remote)
        assertFalse(Page.CLEANUP in remote)
        for (p in listOf(Page.LANGUAGE, Page.STYLE, Page.APP_STYLES, Page.TIMING, Page.BUBBLE, Page.APPS, Page.HISTORY)) assertTrue("$p", p in remote)
    }

    @Test fun localAndHybridShowEveryPage() {
        for (caps in listOf(BuildCaps.LOCAL, BuildCaps.ALL)) {
            assertEquals(Page.entries.filter { it != Page.HUB }, SettingsPages.visiblePages(caps))
        }
    }

    @Test fun onlyPagesWithTypedFieldsHaveSave() {
        // The rule: typed fields → Save (+ discard prompt); switches / choices → apply at once.
        assertEquals(setOf(Page.TIMING, Page.APPS, Page.APP_STYLES), Page.entries.filter { SettingsPages.hasForm(it) }.toSet())
        for (f in Field.entries) assertTrue("$f", SettingsPages.hasForm(SettingsPages.pageOf(f)))
    }

    // ---- form validation ----

    private fun draft(p: Page, base: Settings = s) = SettingsDraft.of(base, p)

    @Test fun freshDraftIsCleanAndCannotSave() {
        val d = draft(Page.TIMING)
        assertFalse(d.dirty); assertFalse(d.canSave())
        assertTrue(d.result().errors.isEmpty())
    }

    @Test fun editMakesDirtyAndUndoMakesClean() {
        val d = draft(Page.TIMING).edit { copy(capMinutes = "10") }
        assertTrue(d.dirty); assertTrue(d.canSave())
        assertEquals(10, d.result().settings!!.capMinutes)
        assertFalse(d.edit { copy(capMinutes = "5") }.dirty)
    }

    @Test fun capOutOfRangeIsAnInlineErrorNotASilentClamp() {
        for (bad in listOf("0", "21", "", "x")) {
            val d = draft(Page.TIMING).edit { copy(capMinutes = bad) }
            assertFalse(bad, d.canSave())
            assertTrue(bad, Field.CAP in d.result().errors)
        }
        assertTrue(draft(Page.TIMING).edit { copy(capMinutes = " 20 ") }.canSave())
    }

    @Test fun endpointZeroOrBlankIsOffAndRangeIsChecked() {
        assertEquals(0, draft(Page.TIMING).edit { copy(endpointMs = "0") }.result().settings!!.endpointSilenceMs)
        assertEquals(0, draft(Page.TIMING).edit { copy(endpointMs = "") }.result().settings!!.endpointSilenceMs)
        assertEquals(300, draft(Page.TIMING).edit { copy(endpointMs = "300") }.result().settings!!.endpointSilenceMs)
        assertEquals(10_000, draft(Page.TIMING).edit { copy(endpointMs = "10000") }.result().settings!!.endpointSilenceMs)
        for (bad in listOf("299", "10001", "abc", "-5")) {
            val r = draft(Page.TIMING).edit { copy(endpointMs = bad) }.result()
            assertNull(bad, r.settings)
            assertTrue(bad, Field.ENDPOINT in r.errors)
        }
    }

    @Test fun decodeWhileSpeakingSavesWithTheTimingPage() {
        val d = draft(Page.TIMING).edit { copy(decodeWhileSpeaking = false) }
        assertTrue(d.canSave())
        assertFalse(d.result().settings!!.decodeWhileSpeaking)
    }

    @Test fun denylistIsTrimmedDedupedAndValidated() {
        val d = draft(Page.APPS).edit { copy(denylist = "  com.example.app \n\ncom.example.app\norg.foo_bar.Baz\n") }
        assertEquals(setOf("com.example.app", "org.foo_bar.Baz"), d.result().settings!!.userDenylist)
        val bad = draft(Page.APPS).edit { copy(denylist = "com.ok.app\nnot a package\nsingle") }.result()
        assertNull(bad.settings)
        val e = bad.errors.getValue(Field.DENYLIST)
        assertTrue(e, "not a package" in e && "single" in e)
        assertTrue(draft(Page.APPS).edit { copy(denylist = "") }.result().settings!!.userDenylist.isEmpty())
    }

    @Test fun appStyleOverridesAreValidatedAndSaved() {
        val d = draft(Page.APP_STYLES).edit { copy(appStyles = "com.example.chat = personal\ncom.x.mail=email") }
        assertTrue(d.dirty)
        assertEquals(
            mapOf("com.example.chat" to dev.flowclone.policy.AppCategorizer.Category.PERSONAL, "com.x.mail" to dev.flowclone.policy.AppCategorizer.Category.EMAIL),
            d.result().settings!!.appBuckets,
        )
        val bad = draft(Page.APP_STYLES).edit { copy(appStyles = "com.ok.app = work\ncom.x.y = loud") }.result()
        assertNull(bad.settings)
        val e = bad.errors.getValue(Field.APP_STYLES)
        assertTrue(e, "com.x.y = loud" in e && "personal, work, email or other" in e)
        val stored = draft(Page.APP_STYLES, s.copy(appBuckets = mapOf("com.z.y" to dev.flowclone.policy.AppCategorizer.Category.WORK)))
        assertEquals("com.z.y = work", stored.draft.appStyles)
        assertFalse(stored.dirty)
        assertFalse("other pages ignore this field", draft(Page.TIMING).edit { copy(appStyles = "x") }.dirty)
    }

    @Test fun storedDenylistShowsSortedAndIsClean() {
        val d = draft(Page.APPS, s.copy(userDenylist = setOf("org.b", "com.a")))
        assertEquals("com.a\norg.b", d.draft.denylist)
        assertFalse(d.dirty)
    }

    @Test fun savingAPageOnlyWritesThatPagesFields() {
        // Another page / an immediate switch changed the store meanwhile: Save must not undo it.
        val d = draft(Page.TIMING).edit { copy(capMinutes = "7") }
        val stored = s.copy(livePreview = false, userDenylist = setOf("com.x.y"), language = "de")
        val out = d.result(stored).settings!!
        assertEquals(7, out.capMinutes)
        assertEquals(stored.copy(capMinutes = 7), out)
    }

    @Test fun rebaseKeepsEditsAndTakesNewStoredValues() {
        val d = draft(Page.APPS).edit { copy(denylist = "com.a.b") }
        val r = d.rebase(s.copy(capMinutes = 9))
        assertEquals("com.a.b", r.draft.denylist)
        assertEquals("9", r.saved.capMinutes)
        assertTrue(r.dirty)
    }

    // ---- immediate changes ----

    @Test fun effectsOfAChange() {
        assertTrue(SettingsPages.effects(s, s.copy(mockEngine = true)).reloadEngine)
        assertTrue(SettingsPages.effects(s, s.copy(llmEnabled = true)).reloadEngine)
        assertTrue(SettingsPages.effects(s, s.copy(asrEngine = AsrEngine.PARAKEET)).reloadEngine)
        assertFalse(SettingsPages.effects(s, s.copy(livePreview = false)).reloadEngine)
        assertFalse("strength is read per session", SettingsPages.effects(s, s.copy(cleanup = CleanupLevel.HIGH)).reloadEngine)
        assertTrue(SettingsPages.effects(s, s.copy(retention = Retention.DAY)).purgeHistory)
        assertFalse(SettingsPages.effects(s, s).purgeHistory)
    }

    @Test fun stricterRetentionAsksFirst() {
        assertTrue(SettingsPages.deletesHistory(Retention.KEEP, Retention.DAY))
        assertTrue(SettingsPages.deletesHistory(Retention.KEEP, Retention.NEVER))
        assertTrue(SettingsPages.deletesHistory(Retention.DAY, Retention.NEVER))
        assertFalse(SettingsPages.deletesHistory(Retention.NEVER, Retention.KEEP))
        assertFalse(SettingsPages.deletesHistory(Retention.DAY, Retention.DAY))
    }

    @Test fun languageChoicesKeepAnUnlistedStoredCode() {
        val base = SettingsPages.languageChoices("en", "English")
        assertEquals(Settings.LANGUAGE_UNSET, base.first().first)
        assertTrue("English" in base.first().second)
        assertEquals(base.size, SettingsPages.languageChoices("fr", "English").size)
        val extra = SettingsPages.languageChoices("sw", "English")
        assertEquals(base.size + 1, extra.size)
        assertEquals("sw" to "Swahili", extra.last())
    }

    // ---- hub subtitles ----

    @Test fun shortModelNames() {
        assertEquals("unified-en-0.6b", SettingsSummary.shortModel("sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8"))
        assertEquals("qwen2.5-1.5b", SettingsSummary.shortModel("qwen2.5-1.5b-instruct-q4_k_m.gguf"))
        assertEquals("base.en", SettingsSummary.shortModel("ggml-base.en.bin"))
        assertEquals("small", SettingsSummary.shortModel("ggml-small-q5_1.bin"))
        assertNull(SettingsSummary.shortModel(null))
    }

    @Test fun languageSubtitle() {
        assertEquals("Phone language (English) · auto-format on", SettingsSummary.language(s, "en"))
        assertEquals("Phone language (auto-detect) · auto-format on", SettingsSummary.language(s, "auto"))
        assertEquals("German · auto-format off", SettingsSummary.language(s.copy(language = "de", spokenCleanup = false), "de"))
        assertEquals("Auto-detect · auto-format on", SettingsSummary.language(s.copy(language = "auto"), "auto"))
    }

    @Test fun engineSubtitle() {
        val pk = s.copy(asrEngine = AsrEngine.PARAKEET)
        assertEquals("Parakeet · unified-en-0.6b", SettingsSummary.engine(pk, whisper = "ggml-base.en.bin", parakeet = "parakeet-unified-en-0.6b-int8"))
        assertEquals("Parakeet · no model folder (whisper is used)", SettingsSummary.engine(pk, whisper = "ggml-base.en.bin", parakeet = null))
        assertEquals("Whisper · base.en", SettingsSummary.engine(s, whisper = "ggml-base.en.bin", parakeet = "x"))
        assertEquals("Whisper · no model imported", SettingsSummary.engine(s, whisper = null, parakeet = null))
    }

    @Test fun cleanupSubtitle() {
        assertEquals("Off · auto-format still runs", SettingsSummary.cleanup(s, "qwen2.5-1.5b-instruct-q4_k_m.gguf"))
        assertEquals("Off · no formatting (auto-format off)", SettingsSummary.cleanup(s.copy(spokenCleanup = false), null))
        // Stored NONE with the switch on runs LIGHT (Settings.effectiveCleanup): show what runs.
        assertEquals("On · qwen2.5-1.5b · Light", SettingsSummary.cleanup(s.copy(llmEnabled = true), "qwen2.5-1.5b-instruct-q4_k_m.gguf"))
        assertEquals("On · no model imported · High", SettingsSummary.cleanup(s.copy(llmEnabled = true, cleanup = CleanupLevel.HIGH), null))
    }

    @Test fun strengthPickerShowsLightForAStoredNone() {
        assertEquals(CleanupLevel.LIGHT, SettingsPages.shownStrength(CleanupLevel.NONE))
        assertEquals(CleanupLevel.MEDIUM, SettingsPages.shownStrength(CleanupLevel.MEDIUM))
    }

    @Test fun timingBubbleAppsHistorySubtitles() {
        assertEquals("Stops after 1.5 s of silence · decode while speaking · 5 min limit", SettingsSummary.timing(s))
        assertEquals("Stops when you tap · 20 min limit", SettingsSummary.timing(s.copy(endpointSilenceMs = 0, decodeWhileSpeaking = false, capMinutes = 20)))
        assertEquals("Stops after 0.3 s of silence · 1 min limit", SettingsSummary.timing(s.copy(endpointSilenceMs = 300, decodeWhileSpeaking = false, capMinutes = 1)))
        assertEquals("Live preview · shrinks to a dot · shake to restore", SettingsSummary.bubble(s))
        assertEquals("Shake to restore", SettingsSummary.bubble(s.copy(livePreview = false, shrinkBubble = false)))
        assertEquals("Preview, dot and shake off", SettingsSummary.bubble(s.copy(livePreview = false, shrinkBubble = false, shakeToReveal = false)))
        assertEquals("None (banking apps are always excluded)", SettingsSummary.apps(s))
        assertEquals("1 app", SettingsSummary.apps(s.copy(userDenylist = setOf("a.b"))))
        assertEquals("2 apps", SettingsSummary.apps(s.copy(userDenylist = setOf("a.b", "c.d"))))
        assertEquals("Keep everything", SettingsSummary.history(Retention.KEEP))
        assertEquals("Last 24 hours", SettingsSummary.history(Retention.DAY))
        assertEquals("Never saved", SettingsSummary.history(Retention.NEVER))
    }

    @Test fun hubWarnings() {
        assertTrue(SettingsSummary.warnings(s, BuildCaps.ALL, whisper = "w", parakeet = null, llm = null).isEmpty())
        assertEquals(
            listOf(SettingsSummary.WARN_MOCK, SettingsSummary.WARN_NO_PARAKEET, SettingsSummary.WARN_NO_LLM),
            SettingsSummary.warnings(s.copy(mockEngine = true, llmEnabled = true, asrEngine = AsrEngine.PARAKEET), BuildCaps.LOCAL, whisper = "w", parakeet = null, llm = null),
        )
        assertEquals(listOf(SettingsSummary.WARN_NO_WHISPER), SettingsSummary.warnings(s, BuildCaps.LOCAL, whisper = null, parakeet = null, llm = null))
        // remote: no on-device engine or LLM, so none of their warnings.
        assertTrue(SettingsSummary.warnings(s.copy(llmEnabled = true, asrEngine = AsrEngine.PARAKEET), BuildCaps.REMOTE, null, null, null).isEmpty())
        // hybrid without a whisper file: the server does the work, so no whisper warning.
        assertTrue(SettingsSummary.warnings(s, BuildCaps.ALL, whisper = null, parakeet = null, llm = null).isEmpty())
    }
}
