package dev.murmur.data

import dev.murmur.core.AsrEngine
import dev.murmur.core.CleanupLevel
import dev.murmur.core.HealthReport
import dev.murmur.core.Rtt
import dev.murmur.data.SettingsPages.Field
import dev.murmur.data.SettingsPages.Page
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
        for (p in listOf(Page.LANGUAGE, Page.STYLE, Page.APP_STYLES, Page.SPEECH, Page.TIMING, Page.BUBBLE, Page.APPS, Page.HISTORY)) assertTrue("$p", p in remote)
    }

    @Test fun localAndHybridShowEveryPage() {
        for (caps in listOf(BuildCaps.LOCAL, BuildCaps.ALL)) {
            assertEquals(Page.entries.filter { it != Page.HUB }, SettingsPages.visiblePages(caps))
        }
    }

    @Test fun onlyPagesWithTypedFieldsHaveSave() {
        // The rule: typed fields → Save (+ discard prompt); switches / choices → apply at once.
        assertEquals(setOf(Page.TIMING, Page.APPS), Page.entries.filter { SettingsPages.hasForm(it) }.toSet())
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

    @Test fun timingSaveLeavesTranscribeWhileISpeakAlone() {
        // "Transcribe while I speak" is a switch on Speech to text now (applies at once).
        val out = draft(Page.TIMING).edit { copy(capMinutes = "9") }.result(s.copy(decodeWhileSpeaking = false)).settings!!
        assertFalse(out.decodeWhileSpeaking)
        assertEquals(9, out.capMinutes)
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

    @Test fun storedDenylistShowsSortedAndIsClean() {
        val d = draft(Page.APPS, s.copy(userDenylist = setOf("org.b", "com.a")))
        assertEquals("com.a\norg.b", d.draft.denylist)
        assertFalse(d.dirty)
    }

    @Test fun savingAPageOnlyWritesThatPagesFields() {
        // Another page / an immediate switch changed the store meanwhile: Save must not undo it.
        val d = draft(Page.TIMING).edit { copy(capMinutes = "7") }
        val stored = s.copy(livePreview = true, userDenylist = setOf("com.x.y"), language = "de")
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
        assertFalse(SettingsPages.effects(s, s.copy(livePreview = true)).reloadEngine)
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
        assertEquals(Settings.LANGUAGE_KEYBOARD, base[0].first)
        assertEquals(Settings.LANGUAGE_UNSET, base[1].first)
        assertTrue("English" in base[1].second)
        assertEquals(base.size, SettingsPages.languageChoices("fr", "English").size)
        val extra = SettingsPages.languageChoices("sw", "English")
        assertEquals(base.size + 1, extra.size)
        assertEquals("sw" to "Swahili", extra.last())
    }

    @Test fun keyboardChoiceSaysWhatTheKeyboardIsNow() {
        assertEquals("Keyboard language · now: Hebrew", SettingsPages.languageChoices("en", "English", "he").first().second)
        assertEquals("Keyboard language · now: unknown (phone language is used)", SettingsPages.keyboardChoice(null))
        assertEquals("the stored choice is listed once", SettingsPages.languageChoices("en", "English").size, SettingsPages.languageChoices(Settings.LANGUAGE_KEYBOARD, "English").size)
    }

    @Test fun hebrewModelRowSubtitle() {
        assertEquals("None imported · Hebrew uses whisper", SettingsSummary.hebrewModel(null))
        assertEquals("Zipformer · used when the language is Hebrew", SettingsSummary.hebrewModel("Zipformer"))
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
        val phone = s.copy(language = Settings.LANGUAGE_UNSET)
        assertEquals("Phone language (English) · auto-format on", SettingsSummary.language(phone, "en"))
        assertEquals("Phone language (auto-detect) · auto-format on", SettingsSummary.language(phone, "auto"))
        assertEquals("Keyboard language (Hebrew) · auto-format on", SettingsSummary.language(s, "he"))
        assertEquals("Keyboard language (auto-detect) · auto-format on", SettingsSummary.language(s, "auto"))
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
        // AndroidSettings.dc.html: "Stops after a 2 s pause · 5 min max", "Banking apps and 2 more", "1 day. Stored only on this phone."
        assertEquals("Stops after a 1.5 s pause · 5 min max", SettingsSummary.timing(s))
        assertEquals("Stops when you tap · 20 min max", SettingsSummary.timing(s.copy(endpointSilenceMs = 0, capMinutes = 20)))
        assertEquals("Stops after a 2 s pause · 1 min max", SettingsSummary.timing(s.copy(endpointSilenceMs = 2000, capMinutes = 1)))
        assertEquals("100% · shrinks to a dot · shake to restore", SettingsSummary.bubble(s))
        assertEquals("100% · live preview · shrinks to a dot · shake to restore", SettingsSummary.bubble(s.copy(livePreview = true)))
        assertEquals("100% · shake to restore", SettingsSummary.bubble(s.copy(shrinkBubble = false)))
        assertEquals("100% · shrinks when idle · shake to restore", SettingsSummary.bubble(s.copy(shrinkToDot = false)))
        assertEquals("75%", SettingsSummary.bubble(s.copy(livePreview = false, shrinkBubble = false, shakeToReveal = false, bubbleScale = 0.75f)))
        assertEquals("Banking apps", SettingsSummary.apps(s))
        assertEquals("Banking apps and 1 more", SettingsSummary.apps(s.copy(userDenylist = setOf("a.b"))))
        assertEquals("Banking apps and 2 more", SettingsSummary.apps(s.copy(userDenylist = setOf("a.b", "c.d"))))
        assertEquals("Until you delete it. Stored only on this phone.", SettingsSummary.keepHistory(Retention.KEEP))
        assertEquals("1 day. Stored only on this phone.", SettingsSummary.keepHistory(Retention.DAY))
        assertEquals("Off. Nothing is saved.", SettingsSummary.keepHistory(Retention.NEVER))
        assertEquals("Keep everything", SettingsSummary.history(Retention.KEEP))
    }

    // ---- Models / Connections rows ----

    private val dev = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = "tok_0123456789abcdef")

    @Test fun speechToTextRowListsTheEnginesThatAreOnInOrder() {
        val caps = BuildCaps.ALL
        assertEquals("Your server, then this phone", SettingsSummary.speech(dev, caps, phoneReady = true))
        assertEquals("This phone, then your server", SettingsSummary.speech(dev.copy(routeOrder = listOf(DevSettings.Route.CLIENT, DevSettings.Route.SERVER)), caps, true))
        assertEquals("This phone only", SettingsSummary.speech(dev.copy(allowServer = false), caps, true))
        assertEquals("Off: no engine is on", SettingsSummary.speech(dev.copy(allowServer = false, allowClient = false), caps, true))
        assertEquals("Cloud, then your server, then this phone", SettingsSummary.speech(OpenRouterPage.enableStt(dev, 1), caps, true))
        // A build without the server sees only the phone, whatever is stored.
        assertEquals("This phone only", SettingsSummary.speech(dev, BuildCaps.LOCAL, true))
    }

    @Test fun speechToTextRowNamesOnlyEnginesThatCanRun() {
        // Owner report: fresh install, no server, no phone model -> the drawer said "Server first → Phone".
        val caps = BuildCaps.ALL
        val fresh = DevSettings.defaults(caps)
        assertEquals("Not set up: add your server or import a speech model", SettingsSummary.speech(fresh, caps, phoneReady = false))
        assertEquals("Not set up: add your server", SettingsSummary.speech(fresh.copy(allowClient = false), caps, false))
        // The cloud counts once consented.
        val cloudNoConsent = fresh.copy(allowCloud = true, routeOrder = DevSettings.withCloudFirst(fresh.routeOrder))
        assertEquals("Not set up: allow OpenRouter, add your server or import a speech model", SettingsSummary.speech(cloudNoConsent, caps, false))
    }

    @Test fun speechToTextRowShowsEnginesThatAreOnButCannotRun() {
        // Review: an engine switched on but not set up is named, not dropped.
        val caps = BuildCaps.ALL
        val fresh = DevSettings.defaults(caps)
        assertEquals("This phone · server not set up", SettingsSummary.speech(fresh, caps, phoneReady = true))
        assertEquals("Your server · no phone model", SettingsSummary.speech(dev, caps, phoneReady = false))
        assertEquals("Cloud · server not set up · no phone model", SettingsSummary.speech(OpenRouterPage.enableStt(fresh, 1), caps, false))
        val cloudNoConsent = dev.copy(allowCloud = true, routeOrder = DevSettings.withCloudFirst(dev.routeOrder))
        assertEquals("Your server, then this phone · OpenRouter not allowed", SettingsSummary.speech(cloudNoConsent, caps, true))
        // A build without the server never names it.
        assertEquals("This phone only", SettingsSummary.speech(fresh, BuildCaps.LOCAL, true))
    }

    @Test fun phoneIsReadyWhenWhatItLoadsIsInstalled() {
        val none = { false }
        val some = { true }
        assertEquals(false, SpeechEngines.phoneReady(Settings(), LoadSet.Default, none))
        assertEquals(true, SpeechEngines.phoneReady(Settings(), LoadSet.Default, some))
        assertEquals(true, SpeechEngines.phoneReady(Settings(mockEngine = true), LoadSet.Default, none))
        // Review: only a Hebrew model, language Hebrew -> the phone loads it, so it is ready.
        val hebrewOnly = LoadSet.of("he", hebrewInstalled = true, keepBoth = false, defaultInstalled = none)
        assertEquals(LoadSet.Language("he"), hebrewOnly)
        assertEquals(true, SpeechEngines.phoneReady(Settings(), hebrewOnly, none))
        // The same model with English: the default model is what loads, and there is none.
        assertEquals(false, SpeechEngines.phoneReady(Settings(), LoadSet.of("en", hebrewInstalled = true, keepBoth = false, defaultInstalled = none), none))
        assertEquals(true, SpeechEngines.phoneReady(Settings(), LoadSet.Both, none))
    }

    @Test fun aiCleanupRowNamesStrengthEnginesAndRules() {
        val caps = BuildCaps.ALL
        val r = DevSettings.CleanupRoute.entries
        val noServer = dev.withCleanup(DevSettings.CleanupRoute.SERVER, false)
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, noServer, caps))
        assertEquals("Off", SettingsSummary.cleanupRow(s.copy(spokenCleanup = false), noServer, caps))
        assertEquals("Light · This phone, then rules", SettingsSummary.cleanupRow(s.copy(llmEnabled = true), noServer, caps))
        val cloud = OpenRouterPage.enableCleanup(noServer, 1).copy(cleanupOrder = listOf(r[2], r[0], r[1]))
        assertEquals("Medium · Cloud, then rules", SettingsSummary.cleanupRow(s.copy(cleanup = CleanupLevel.MEDIUM), cloud, caps))
        assertEquals("High · Cloud, then this phone, then rules", SettingsSummary.cleanupRow(s.copy(llmEnabled = true, cleanup = CleanupLevel.HIGH), cloud, caps))
        assertEquals("Light · This phone, then cloud", SettingsSummary.cleanupRow(s.copy(llmEnabled = true, spokenCleanup = false), cloud.copy(cleanupOrder = DevSettings.DEFAULT_CLEANUP_ORDER), caps))
        // No consent recorded: the cloud is not used, so not named.
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, noServer.copy(allowCloudCleanup = true), caps))
        // D36: your server (on by default) cleans even with the phone's AI off; named only once set up.
        assertEquals("Light · Your server, then rules", SettingsSummary.cleanupRow(s, dev, caps))
        assertEquals("Light · This phone, then your server, then rules", SettingsSummary.cleanupRow(s.copy(llmEnabled = true), dev, caps))
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, DevSettings.defaults(caps), caps))
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, dev.effective(BuildCaps.LOCAL), BuildCaps.LOCAL))
        // A server known to have no AI model is skipped by the router, so not named.
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, dev, caps, serverLlm = false))
        assertEquals("Light · Your server, then rules", SettingsSummary.cleanupRow(s, dev, caps, serverLlm = true))
    }

    @Test fun aiCleanupRowNamesACustomServerOnlyWhenUsable() {
        FakeCustomAddress.install()
        val noServer = dev.withCleanup(DevSettings.CleanupRoute.SERVER, false)
        val custom = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "gpt-4o-mini")
        val on = OpenRouterPage.enableCleanup(noServer.copy(cloud = noServer.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom)), 1)
        assertEquals("Light · Cloud, then rules", SettingsSummary.cleanupRow(s, on, BuildCaps.ALL))
        // D52: not set up (no model) sends nothing: neither named nor counted as an engine off the phone.
        val broken = OpenRouterPage.withCustom(on) { copy(model = "") }
        assertTrue(broken.cloud.customNotSetUp)
        assertEquals("Rules only", SettingsSummary.cleanupRow(s, broken, BuildCaps.ALL))
        assertFalse(broken.offPhoneCleanup)
    }

    @Test fun serverRowFromTheLastHealthProbe() {
        val caps = BuildCaps.ALL
        val ok = HealthReport(ok = true, rttMs = Rtt(30, 41, 60))
        assertEquals("Connected · 41 ms", SettingsSummary.server(dev, caps, ok))
        assertEquals("Not reachable", SettingsSummary.server(dev, caps, HealthReport(ok = false, error = "connect_failed")))
        assertEquals("Checking…", SettingsSummary.server(dev, caps, null, checking = true))
        assertEquals("Not checked yet", SettingsSummary.server(dev, caps, null))
        assertEquals("Not set up", SettingsSummary.server(DevSettings.defaults(caps), caps, ok))
        assertEquals("Off · 100.64.0.10", SettingsSummary.server(dev.copy(allowServer = false), caps, ok))
        assertEquals("Not in this build", SettingsSummary.server(dev, BuildCaps.LOCAL, ok))
    }

    @Test fun cloudRowNamesTheProviderKeyAndThisMonthsCost() {
        FakeCustomAddress.install()
        val d = DevSettings.defaults(BuildCaps.ALL)
        val u = CloudUsage("2026-10", listOf(CloudUsage.ProviderUsage("openrouter", costUsd = 0.30), CloudUsage.ProviderUsage("openrouter", costUsd = 0.12, purpose = CloudUsage.PURPOSE_CLEANUP)))
        assertEquals("OpenRouter · key saved · \$0.42 this month", SettingsSummary.cloud(d, true, u))
        assertEquals("OpenRouter · no key · \$0 this month", SettingsSummary.cloud(d, false, CloudUsage()))
        // D52: the proxy's spend is not OpenRouter's.
        val custom = u.copy(providers = u.providers + CloudUsage.ProviderUsage(CustomServer.ID, requests = 4, costUsd = 3.0, purpose = CloudUsage.PURPOSE_CLEANUP))
        assertEquals("OpenRouter · key saved · \$0.42 this month", SettingsSummary.cloud(d, true, custom))
        // D78: LiteLLM is named with its host and the cost it reported (never OpenRouter's).
        val ll = d.copy(cloud = d.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = CustomCleanupSettings("http://10.0.0.5:4000/v1", "gpt-4o-mini")))
        assertEquals("LiteLLM · 10.0.0.5 · \$3.00 this month", SettingsSummary.cloud(ll, false, custom))
        assertEquals("LiteLLM · 10.0.0.5", SettingsSummary.cloud(ll, true, u))
        assertEquals("LiteLLM · not set up", SettingsSummary.cloud(OpenRouterPage.withCustom(ll) { copy(model = "") }, true, u))
    }

    @Test fun cloudSpeechRowIsNotAvailableWithLiteLlm() {
        val d = DevSettings.defaults(BuildCaps.ALL)
        assertEquals("Sends audio · asks first", SettingsSummary.cloudEngine(d, hasKey = true))
        val ll = OpenRouterPage.withProvider(OpenRouterPage.enableStt(d, 1), CleanupProvider.CUSTOM)
        assertEquals(SettingsSummary.CLOUD_STT_NOT_AVAILABLE, SettingsSummary.cloudEngine(ll, hasKey = true))
        assertTrue(SettingsSummary.CLOUD_STT_NOT_AVAILABLE.startsWith("Not available"))
        assertEquals("Cloud", SettingsSummary.engineName(DevSettings.Route.CLOUD))
    }

    @Test fun engineLines() {
        assertEquals("Parakeet · works offline", SettingsSummary.phoneEngine(s.copy(asrEngine = AsrEngine.PARAKEET), "w.bin", "pk"))
        assertEquals("Whisper · works offline", SettingsSummary.phoneEngine(s.copy(asrEngine = AsrEngine.PARAKEET), "w.bin", null))
        assertEquals("No model imported", SettingsSummary.phoneEngine(s, null, null))
        assertEquals("Sends audio · asks first", SettingsSummary.cloudEngine(dev, hasKey = false))
        val on = OpenRouterPage.enableStt(dev, 1)
        assertEquals("Sends audio · " + OpenRouter.DEFAULT_STT_MODEL.substringAfter('/'), SettingsSummary.cloudEngine(on, hasKey = true))
        assertEquals("Sends audio · " + OpenRouter.DEFAULT_STT_MODEL.substringAfter('/') + " · no key", SettingsSummary.cloudEngine(on, hasKey = false))
    }

    @Test fun phoneModelCardLine() {
        val oct20 = java.time.LocalDate.of(2026, 10, 20).atStartOfDay(java.time.ZoneOffset.UTC).toInstant().toEpochMilli()
        val date = { ms: Long -> java.time.format.DateTimeFormatter.ofPattern("MMM d", java.util.Locale.US).withZone(java.time.ZoneOffset.UTC).format(java.time.Instant.ofEpochMilli(ms)) }
        assertEquals("English · 640 MB · imported Oct 20", SettingsSummary.modelLine(AsrEngine.PARAKEET, "parakeet-tdt-0.6b-v2-int8", 640_000_000, oct20, date))
        assertEquals("English · 60 MB · imported Oct 20", SettingsSummary.modelLine(AsrEngine.WHISPER, "ggml-base.en-q5_1.bin", 60_000_000, oct20, date))
        assertEquals("Many languages · 60 MB", SettingsSummary.modelLine(AsrEngine.WHISPER, "ggml-base-q5_1.bin", 60_000_000, 0, date))
    }

    @Test fun bubbleSizeDefaultsTo100PercentAndStaysOnTheSliderGrid() {
        assertEquals(1f, Settings().bubbleScale)
        assertEquals(100, SettingsSummary.bubbleSizePercent(Settings().bubbleScale))
        assertEquals(0.75f, Settings.bubbleScaleOf(0.1f))
        assertEquals(1.5f, Settings.bubbleScaleOf(9f))
        assertEquals(1f, Settings.bubbleScaleOf(Float.NaN))
        assertEquals(1.05f, Settings.bubbleScaleOf(1.04f))
        assertEquals(0.95f, Settings.bubbleScaleOf(95 / 100f))
        assertEquals(150, SettingsSummary.bubbleSizePercent(1.5f))
        assertEquals(75, SettingsSummary.bubbleSizePercent(0.75f))
        assertEquals(85, SettingsSummary.bubbleSizePercent(0.85f))
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
