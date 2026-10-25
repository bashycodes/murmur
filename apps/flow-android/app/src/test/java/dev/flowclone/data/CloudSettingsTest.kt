package dev.flowclone.data

import dev.flowclone.core.RouteInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D28: the one OpenRouter config (schema 2), its two consents, the key at rest and the wording. */
class CloudSettingsTest {
    private val now = 1_792_540_800_000L

    @Test fun defaultsAreThePrivateOnes() {
        val c = CloudSettings()
        assertTrue(c.privacy.zdr); assertEquals("deny", c.privacy.dataCollection)
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, c.stt.model); assertEquals("openai/whisper-large-v3-turbo", c.stt.model)
        assertEquals(OpenRouter.DEFAULT_CLEANUP_MODEL, c.cleanup.model)
        assertEquals(2_500L, c.cleanup.timeoutMs); assertEquals("latency", c.cleanup.sort)
        assertFalse(c.cleanup.sendSurroundingText || c.cleanup.sendScreenWords || c.cleanup.sendAppName)
        assertFalse(c.stt.consentOk); assertFalse(c.cleanup.consentOk)
        assertNull(c.monthlyBudgetUsd)
        assertEquals(2, c.toJson().getInt("schema"))
    }

    @Test fun consentsAreSeparateAndNamedForOpenRouter() {
        val c = CloudSettings()
        val stt = c.copy(stt = c.stt.withConsent(now))
        assertTrue(stt.stt.consentOk); assertFalse("STT consent never counts for cleanup", stt.cleanup.consentOk)
        assertEquals("openrouter.ai", stt.stt.consentHost)
        val both = stt.copy(cleanup = stt.cleanup.withConsent(now + 1))
        assertTrue(both.cleanup.consentOk)
        assertFalse(both.copy(stt = both.stt.withoutConsent()).stt.consentOk)
        // A consent recorded for another host (a hand-edited prefs file) is no consent.
        assertFalse(c.stt.copy(consentHost = "example.com", consentAtMs = now).consentOk)
    }

    @Test fun jsonRoundTripAndLegacyReadsAsDefault() {
        val c = CloudSettings(
            privacy = CloudPrivacy(zdr = false, dataCollection = "allow"),
            stt = SttSettings(model = "elevenlabs/scribe-v2", language = "de", sendDictionary = false).withConsent(now),
            cleanup = CleanupSettings(model = "meta-llama/llama-3.1-8b-instruct", timeoutMs = 4000, sendAppName = true).withConsent(now),
            monthlyBudgetUsd = 2.5,
        )
        assertEquals(c, CloudSettings.fromJson(JSONObject(c.toJson().toString())))
        val legacy = JSONObject("""{"active":"groq-1","providers":[{"id":"groq-1","adapter":"openai_compat"}],"monthly_budget_usd":5}""")
        assertTrue(CloudSettings.isLegacy(legacy))
        assertEquals(CloudSettings(), CloudSettings.fromJson(legacy))
        assertTrue("no schema = 1", CloudSettings.isLegacy(JSONObject("""{"monthly_budget_usd":5}""")))
        assertFalse(CloudSettings.isLegacy(c.toJson()))
        // openRouterJson is the OpenRouterConfig nativeProbeCloud takes: no host, no key.
        val or = c.openRouterJson()
        assertTrue(or.has("stt") && or.has("cleanup") && or.has("privacy"))
        assertFalse(or.toString().contains("base_url"))
    }

    @Test fun modelIdRuleMatchesTheRouter() {
        for (ok in listOf("openai/whisper-large-v3-turbo", "meta-llama/llama-3.3-70b-instruct:nitro", "x/y.z~1_2-3", "a")) assertTrue(ok, OpenRouter.validModelId(ok))
        for (bad in listOf("", "Openai/Whisper", "a b", "x/y:Nitro", "x/y:", "x".repeat(129), "https://openrouter.ai/x")) assertFalse(bad, OpenRouter.validModelId(bad))
        assertNull(OpenRouter.modelIdError(" openai/whisper-large-v3 "))
        assertEquals("Model ids are lower case", OpenRouter.modelIdError("OpenAI/x"))
    }

    @Test fun inputs() {
        assertEquals(2.5, CloudInput.budget("$2.50").value!!, 0.0)
        assertNull(CloudInput.budget(" ").value); assertNull(CloudInput.budget(" ").error)
        assertTrue(CloudInput.budget("-1").error != null)
        assertEquals("5", CloudInput.budgetText(5.0)); assertEquals("2.5", CloudInput.budgetText(2.5)); assertEquals("", CloudInput.budgetText(null))
        assertEquals(800L, CloudInput.timeout("800").value); assertTrue(CloudInput.timeout("799").error != null); assertTrue(CloudInput.timeout("8001").error != null)
        assertEquals("pt-br", CloudInput.language(" PT-BR ").value); assertTrue(CloudInput.language("english").error != null)
    }

    @Test fun consentDialogsSayWhatLeaves() {
        val stt = CloudText.sttConsentBody(SttSettings())
        assertTrue(stt, stt.contains("audio") && stt.contains("openrouter.ai") && stt.contains("dictionary words"))
        assertFalse(CloudText.sttConsentBody(SttSettings(sendDictionary = false)).contains("dictionary"))
        val cl = CloudText.cleanupConsentBody(CleanupSettings())
        // Spec §2.5 wording.
        assertTrue(cl, cl.startsWith("After each dictation, the text (and your dictionary words) is sent to OpenRouter (openrouter.ai)"))
        assertTrue(cl.contains("Your voice is not sent unless Cloud speech-to-text is also on"))
        assertFalse("minimal scope by default", cl.contains("around the cursor") || cl.contains("on your screen") || cl.contains("app's name"))
        val all = CloudText.cleanupSends(CleanupSettings(sendSurroundingText = true, sendScreenWords = true, sendAppName = true))
        assertTrue(all, all.contains("the text around the cursor") && all.contains("words on your screen") && all.endsWith("and the app's name"))
    }

    @Test fun usageIsSplitByPurpose() {
        val u = CloudUsage.decode(javaClass.classLoader!!.getResource("cloud_usage.json")!!.readText())
        assertEquals(0.0234, u.costFor(CloudUsage.PURPOSE_STT), 1e-9)
        assertEquals(0.014, u.costFor(CloudUsage.PURPOSE_CLEANUP), 1e-9)
        assertTrue(u.estimated(CloudUsage.PURPOSE_CLEANUP)); assertFalse(u.estimated(CloudUsage.PURPOSE_STT))
        val lines = CloudText.usageLines(u, CloudSettings(monthlyBudgetUsd = 5.0))
        assertEquals("Speech \$0.02 · Cleanup ≈ \$0.01 · this month (2026-10)", lines[0])
        assertEquals("42 speech requests (5.2 min) · 20 cleanup requests", lines[1])
        assertEquals("Budget \$5.00/month: \$0.04 used", lines[2])
        // A pre-D28 entry has no purpose: speech.
        assertEquals(CloudUsage.PURPOSE_STT, CloudUsage.decode("""{"month":"m","providers":[{"provider_id":"x","cost_usd":1}]}""").providers.single().purpose)
        assertEquals(u, CloudUsage.decode(u.toJson().toString()))
    }

    @Test fun routeWording() {
        val r = RouteInfo("cloud", provider = "openrouter", model = "openai/whisper-large-v3-turbo", costUsd = 0.000011, llmEngine = "cloud",
            llmModel = "meta-llama/llama-3.3-70b-instruct", llmCostUsd = 0.0007)
        assertEquals("cloud · OpenRouter · openai/whisper-large-v3-turbo · \$0.0000", CloudText.engineRow(r))
        assertEquals("AI cleanup: OpenRouter · llama-3.3-70b-instruct · \$0.0007", CloudText.cleanupRow(r))
        assertNull(CloudText.cleanupRow(RouteInfo("local")))
        assertEquals("When your server is unreachable, audio goes to OpenRouter.",
            CloudText.orderWarning(listOf(DevSettings.Route.SERVER, DevSettings.Route.CLOUD)))
        assertNull(CloudText.orderWarning(listOf(DevSettings.Route.CLOUD, DevSettings.Route.SERVER)))
    }

    // ---- the key at rest (C3) ----

    private val key = "sk-or-LEAKCHECK-android-0123456789abcd"

    @Test fun oneKeySealedAndNeverInSettings() {
        val kv = MemKV()
        val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("flow_dev_token", fake))
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(DevSettings.defaults(BuildCaps.ALL), now), now, BuildCaps.ALL)
            .let { it.copy(cloud = it.cloud.copy(monthlyBudgetUsd = 3.0)) }
        store.save(s)
        keys.set(OpenRouter.ID, " $key ")
        assertEquals(key, keys.get(OpenRouter.ID))
        assertEquals("••••abcd", keys.masked(OpenRouter.ID))
        assertEquals(s, store.load())
        assertFalse(kv.m.values.any { it.toString().contains("LEAKCHECK") })
        assertFalse(s.toRouterConfigJson().toString().contains("LEAKCHECK"))
        assertFalse(s.toString().contains("LEAKCHECK"))
        assertEquals(setOf("flow_cloud_openrouter"), fake.keys.keys.filter { it.startsWith("flow_cloud_") }.toSet())
        keys.set(OpenRouter.ID, "")
        assertNull(keys.get(OpenRouter.ID)); assertFalse("flow_cloud_openrouter" in fake.keys)
    }

    @Test fun moveReSealsUnderTheNewVaultAndDestroysTheOld() {
        val kv = MemKV(); val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        keys.set("openrouter-1", key)
        keys.move("openrouter-1", OpenRouter.ID)
        assertEquals(key, keys.get(OpenRouter.ID))
        assertFalse(keys.has("openrouter-1")); assertFalse("flow_cloud_openrouter-1" in fake.keys)
        keys.move("openrouter-1", OpenRouter.ID) // idempotent
        assertEquals(key, keys.get(OpenRouter.ID))
    }

    @Test fun storeKeepsCleanupFieldsAndALocalBuildMasksThem() {
        val kv = MemKV(); val fake = FakeVaultKeys()
        val s = OpenRouterPage.enableCleanup(DevSettings.defaults(BuildCaps.ALL), now, BuildCaps.ALL)
        DevStore(kv, BuildCaps.ALL, SecretVault("t", fake)).save(s)
        assertEquals(listOf(DevSettings.CleanupRoute.CLOUD, DevSettings.CleanupRoute.LOCAL), DevStore(kv, BuildCaps.ALL, SecretVault("t", fake)).load().cleanupOrder)
        val local = DevStore(kv, BuildCaps.LOCAL, SecretVault("t", fake)).load()
        assertFalse(local.allowCloudCleanup)
        // …and a local build's router JSON carries no cloud field at all (C2).
        val j = JSONObject(local.routerConfigFor(BuildCaps.LOCAL))
        for (k in listOf("cloud", "allow_cloud", "allow_cloud_cleanup", "cleanup_order", "order")) assertFalse(k, j.has(k))
    }
}
