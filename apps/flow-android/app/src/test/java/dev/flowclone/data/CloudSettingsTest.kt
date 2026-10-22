package dev.flowclone.data

import dev.flowclone.core.RouteInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D18 (docs/cloud-stt.md §3.1, §8): cloud settings, consent, route order, keys at rest, wording. */
class CloudSettingsTest {
    private val groq = CloudPresets.of("groq").newProvider(emptyList())
    private val consented = groq.withConsent(1_000)

    /** C2: the flavour's BuildConfig says cloud exactly where the network flavours are (hybrid, remote). */
    @Test fun buildConfigHasCloudOnlyInNetworkFlavours() {
        val c = BuildCaps.current()
        assertEquals(c.remote, c.cloud)
        assertEquals(c.flavor != "local", c.cloud)
    }

    // ---- consent ----

    @Test fun consentNamesTheCurrentHost() {
        assertFalse(groq.consentOk)
        assertTrue(consented.consentOk)
        assertEquals("api.groq.com", consented.consentHost)
        assertEquals(1_000L, consented.consentAtMs)
    }

    @Test fun changingTheHostOrAdapterInvalidatesConsent() {
        val otherHost = consented.edited(consented.copy(baseUrl = "https://api.example.com/openai/v1"))
        assertFalse(otherHost.consentOk); assertEquals("", otherHost.consentHost); assertEquals(0L, otherHost.consentAtMs)
        val otherAdapter = consented.edited(consented.copy(adapter = "deepgram"))
        assertFalse(otherAdapter.consentOk)
        // Same host (path, port, case) or a new model keeps it.
        assertTrue(consented.edited(consented.copy(model = "whisper-large-v3")).consentOk)
        assertTrue(consented.edited(consented.copy(baseUrl = "https://API.groq.com:443/openai/v2")).consentOk)
        // Even a raw edit that slips the host past edited(): consentOk compares hosts again (like the router).
        assertFalse(consented.copy(baseUrl = "https://evil.example/v1").consentOk)
    }

    @Test fun formSaveReportsVoidedConsent() {
        val r = CloudProviderForm.validate(CloudProviderForm.Input.of(consented).copy(baseUrl = "https://api.other.com/v1"))
        assertNotNull(r.provider); assertTrue(r.consentVoided); assertFalse(r.provider!!.consentOk)
        val same = CloudProviderForm.validate(CloudProviderForm.Input.of(consented).copy(model = "whisper-large-v3", language = "DE"))
        assertFalse(same.consentVoided); assertTrue(same.provider!!.consentOk); assertEquals("de", same.provider!!.language)
        assertNotNull(CloudProviderForm.validate(CloudProviderForm.Input.of(groq).copy(baseUrl = "http://api.groq.com/v1")).error)
        assertNotNull(CloudProviderForm.validate(CloudProviderForm.Input.of(groq).copy(model = " ")).error)
        assertNotNull(CloudProviderForm.validate(CloudProviderForm.Input.of(groq).copy(timeoutBaseMs = "999")).error)
        assertNotNull(CloudProviderForm.validate(CloudProviderForm.Input.of(groq).copy(language = "english")).error)
    }

    @Test fun settingsUpsertKeepsOrClearsConsent() {
        val s = CloudSettings().upsert(groq)
        assertEquals("groq-1", s.active)
        val withConsent = s.replace(consented)
        assertTrue(withConsent.upsert(consented.copy(model = "whisper-large-v3")).activeProvider!!.consentOk)
        assertFalse(withConsent.upsert(consented.copy(baseUrl = "https://x.example/v1")).activeProvider!!.consentOk)
        val two = withConsent.upsert(CloudPresets.of("openrouter").newProvider(listOf("groq-1")))
        assertEquals(listOf("groq-1", "openrouter-1"), two.providers.map { it.id })
        assertEquals("openrouter-1", two.remove("groq-1").active)
        assertEquals("", two.remove("groq-1").remove("openrouter-1").active)
    }

    // ---- validation (twin of CloudProviderConfig::validate) ----

    @Test fun hostAndValidationRules() {
        assertEquals("openrouter.ai", CloudRules.host("https://OpenRouter.ai/api/v1"))
        assertEquals("::1", CloudRules.host("http://[::1]:8000/v1"))
        assertEquals("192.168.1.5", CloudRules.host("http://192.168.1.5:8000"))
        assertNull(CloudRules.host("ftp://x"))
        assertNull("no credentials in URLs", CloudRules.host("https://user:pw@host/v1"))
        for (p in CloudPresets.ALL.filter { it.id != "custom" }) assertNull(p.id, CloudRules.validate(p.newProvider(emptyList())))
        assertNotNull("custom needs a URL + model", CloudRules.validate(CloudPresets.of("custom").newProvider(emptyList())))
        val selfHosted = CloudPresets.of("custom").newProvider(emptyList()).copy(baseUrl = "http://100.64.0.10:8000/v1", model = "Systran/faster-whisper-small")
        assertNull("http:// is fine for a tailnet speaches", CloudRules.validate(selfHosted))
        assertNotNull(CloudRules.validate(selfHosted.copy(baseUrl = "http://speech.example.com/v1")))
        assertNotNull(CloudRules.validate(groq.copy(id = "Bad_Id")))
        assertNotNull(CloudRules.validate(groq.copy(adapter = "chat")))
        assertEquals("groq-2", CloudRules.newId("groq", listOf("groq-1")))
    }

    @Test fun presetsCoverTheSpec() {
        assertEquals(listOf("openrouter", "elevenlabs", "groq", "openai", "deepgram", "assemblyai", "custom"), CloudPresets.ALL.map { it.id })
        assertEquals("gpt_transcribe", CloudPresets.of("openai").newProvider(emptyList()).dialect)
        assertTrue(CloudPresets.of("openrouter").newProvider(emptyList()).zeroRetention)
        assertFalse("ElevenLabs ZDR is enterprise only: default off", CloudPresets.of("elevenlabs").newProvider(emptyList()).zeroRetention)
        assertTrue(CloudPresets.of("custom").editableBaseUrl)
    }

    // ---- routing (C1) ----

    @Test fun cloudIsNeverImpliedAndNeverInALocalBuild() {
        assertEquals(DevSettings.DEFAULT_ORDER, DevSettings.parseOrder("server,client"))
        assertEquals(listOf(DevSettings.Route.CLOUD, DevSettings.Route.CLIENT, DevSettings.Route.SERVER), DevSettings.parseOrder("cloud,client"))
        assertEquals(listOf(DevSettings.Route.CLOUD, DevSettings.Route.CLIENT, DevSettings.Route.SERVER), DevSettings.withCloudFirst(listOf(DevSettings.Route.CLIENT, DevSettings.Route.SERVER)))
        val s = DevSettings(allowServer = false, allowClient = true, allowCloud = true, routeOrder = DevSettings.withCloudFirst(DevSettings.DEFAULT_ORDER),
            cloud = CloudSettings(active = "groq-1", providers = listOf(consented)))
        assertTrue(s.cloudRouted); assertTrue(s.cloudFirst)
        val local = s.effective(BuildCaps.LOCAL)
        assertFalse(local.allowCloud); assertFalse(DevSettings.Route.CLOUD in local.routeOrder); assertTrue(local.cloud.isDefault)
        assertFalse(s.copy(allowCloud = false).cloudRouted)
        assertFalse("allowed but not ordered = not a route", s.copy(routeOrder = DevSettings.DEFAULT_ORDER).cloudRouted)
        // Cloud-only is a usable setup in a cloud build, "both off" in a local one.
        val cloudOnly = s.copy(allowClient = false)
        assertFalse(cloudOnly.bothOff)
        assertTrue(cloudOnly.effective(BuildCaps.LOCAL).bothOff)
        assertFalse(s.copy(routeOrder = listOf(DevSettings.Route.CLIENT, DevSettings.Route.CLOUD, DevSettings.Route.SERVER)).cloudFirst)
    }

    @Test fun formWarnsAboutUnusableCloudAndOrder() {
        val base = DevSettingsForm.Input.of(DevSettings.defaults(BuildCaps.ALL))
        val noProvider = DevSettingsForm.validate(base.copy(allowCloud = true), BuildCaps.ALL)
        assertTrue(DevSettingsForm.CLOUD_NO_PROVIDER in noProvider.warnings)
        val noConsent = DevSettingsForm.validate(base.copy(allowCloud = true, cloud = CloudSettings("groq-1", listOf(groq)),
            routeOrder = DevSettings.withCloudFirst(DevSettings.DEFAULT_ORDER)), BuildCaps.ALL)
        assertTrue(DevSettingsForm.CLOUD_NO_CONSENT in noConsent.warnings)
        val behindServer = DevSettingsForm.validate(base.copy(allowCloud = true, cloud = CloudSettings("groq-1", listOf(consented)),
            routeOrder = listOf(DevSettings.Route.SERVER, DevSettings.Route.CLOUD, DevSettings.Route.CLIENT)), BuildCaps.ALL)
        assertTrue(behindServer.warnings.toString(), "When your server is unreachable, audio goes to Groq." in behindServer.warnings)
        val notOrdered = DevSettingsForm.validate(base.copy(allowCloud = true, cloud = CloudSettings("groq-1", listOf(consented))), BuildCaps.ALL)
        assertTrue(DevSettingsForm.CLOUD_NOT_IN_ORDER in notOrdered.warnings)
        // Budget: amount or empty.
        assertEquals(2.5, DevSettingsForm.validate(base.copy(cloudBudget = "$2.50"), BuildCaps.ALL).settings!!.cloud.monthlyBudgetUsd!!, 0.0)
        assertNull(DevSettingsForm.validate(base.copy(cloudBudget = " "), BuildCaps.ALL).settings!!.cloud.monthlyBudgetUsd)
        assertTrue(DevSettingsForm.Field.CLOUD_BUDGET in DevSettingsForm.validate(base.copy(cloudBudget = "-1"), BuildCaps.ALL).errors)
        // A local build can't switch the cloud on, but keeps stored providers (flavour switch keeps prefs).
        val local = DevSettingsForm.validate(base.copy(allowCloud = true, cloud = CloudSettings("groq-1", listOf(consented))), BuildCaps.LOCAL).settings!!
        assertFalse(local.allowCloud); assertEquals(1, local.cloud.providers.size)
    }

    // ---- keys at rest (C3) ----

    private class MemKV : KeyValue {
        val m = HashMap<String, Any?>()
        override fun getString(k: String) = m[k] as String?
        override fun getBoolean(k: String, d: Boolean) = m[k] as Boolean? ?: d
        override fun getLong(k: String, d: Long) = m[k] as Long? ?: d
        override fun contains(k: String) = k in m
        override fun edit(block: KeyValue.Editor.() -> Unit) = object : KeyValue.Editor {
            override fun putString(k: String, v: String?) { if (v == null) m.remove(k) else m[k] = v }
            override fun putBoolean(k: String, v: Boolean) { m[k] = v }
            override fun putLong(k: String, v: Long) { m[k] = v }
            override fun remove(k: String) { m.remove(k) }
            override fun clear() = m.clear()
        }.block()
    }

    private val key = "sk-LEAKCHECK-android-0123456789abcd"

    @Test fun keysAreSealedPerProviderAndNeverInSettings() {
        val kv = MemKV()
        val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("flow_dev_token", fake))
        val s = DevSettings.defaults(BuildCaps.ALL).copy(allowCloud = true, routeOrder = DevSettings.withCloudFirst(DevSettings.DEFAULT_ORDER),
            cloud = CloudSettings("groq-1", listOf(consented), monthlyBudgetUsd = 3.0))
        store.save(s)
        keys.set("groq-1", " $key ")
        assertEquals(key, keys.get("groq-1"))
        assertEquals("••••abcd", keys.masked("groq-1"))
        assertEquals(s, store.load())
        // Nothing stored or serialised contains the key in clear.
        assertFalse(kv.m.values.any { it.toString().contains("LEAKCHECK") })
        assertFalse(s.toRouterConfigJson().toString().contains("LEAKCHECK"))
        assertFalse(s.toString().contains("LEAKCHECK"))
        assertTrue("flow_cloud_groq-1" in fake.keys)
        // Delete: ciphertext and its Keystore key go; other providers are untouched.
        keys.set("openrouter-1", "sk-or-other")
        keys.remove("groq-1")
        assertNull(keys.get("groq-1")); assertFalse(keys.has("groq-1")); assertFalse("flow_cloud_groq-1" in fake.keys)
        assertEquals("sk-or-other", keys.get("openrouter-1"))
        keys.set("openrouter-1", "")
        assertNull("blank clears", keys.get("openrouter-1"))
    }

    @Test fun usageSurvivesResetToDefaults() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("t", FakeVaultKeys()))
        store.saveCloud(allowCloud = true, cloud = CloudSettings("groq-1", listOf(consented)), routeOrder = DevSettings.withCloudFirst(DevSettings.DEFAULT_ORDER))
        assertTrue(store.load().cloudRouted)
        store.cloudUsageJson = """{"month":"2026-10","providers":[]}"""
        store.reset()
        assertEquals("""{"month":"2026-10","providers":[]}""", store.cloudUsageJson)
        assertFalse(store.load().allowCloud); assertTrue(store.load().cloud.isDefault)
        // A local build reads the cloud toggle as off.
        store.saveCloud(allowCloud = true, cloud = CloudSettings("groq-1", listOf(consented)))
        assertFalse(DevStore(kv, BuildCaps.LOCAL, SecretVault("t", FakeVaultKeys())).load().allowCloud)
    }

    // ---- user-facing text (§8.3) ----

    @Test fun consentDialogNamesProviderAndHost() {
        assertEquals("Send audio to Groq?", CloudText.consentTitle(groq))
        assertTrue(CloudText.consentBody(groq).contains("Groq (api.groq.com)"))
        assertTrue(CloudText.consentBody(groq).contains("You can turn this off at any time."))
        assertEquals("Send audio to Groq", CloudText.consentButton(groq))
        assertTrue(CloudText.banner(null).startsWith("Off by default."))
    }

    /** Review P3: the banner names what is actually sent with the audio. */
    @Test fun consentBannerFollowsTheProvidersToggles() {
        assertTrue(CloudText.banner(groq.copy(sendDictionary = true, sendScreenWords = false)).contains("along with your dictionary words."))
        assertTrue(CloudText.banner(groq.copy(sendDictionary = true, sendScreenWords = true)).contains("your dictionary words and words on your screen"))
        assertTrue(CloudText.banner(groq.copy(sendDictionary = false, sendScreenWords = true)).contains("along with words on your screen."))
        val bare = CloudText.banner(groq.copy(sendDictionary = false, sendScreenWords = false))
        assertTrue(bare, bare.contains("Groq (api.groq.com). It is processed"))
        assertFalse(bare.contains("dictionary"))
    }

    @Test fun fallbackMessagesAndUsage() {
        val authLocal = RouteInfo(engine = "local", fallback = "auth_failed")
        assertEquals("Cloud STT failed (key rejected); used on-device", CloudText.fallbackMessage(authLocal, cloudWasPrimary = true))
        assertNull("a server auth failure is not the cloud's", CloudText.fallbackMessage(authLocal, cloudWasPrimary = false))
        assertEquals("Cloud STT failed (out of credits); used your server", CloudText.fallbackMessage(RouteInfo("remote", "quota_exceeded"), false))
        assertEquals("Cloud STT failed (monthly budget reached); used on-device", CloudText.fallbackMessage(RouteInfo("local", "budget_exceeded"), false))
        assertNull(CloudText.fallbackMessage(RouteInfo("cloud"), true))
        assertNull(CloudText.fallbackMessage(RouteInfo("local"), true))
        assertEquals("no internet / base URL wrong", CloudText.hint("connect_failed"))
        assertEquals("set a key and accept the consent", CloudText.hint("cloud_not_configured"))
        assertEquals("cloud · Groq · whisper-large-v3-turbo · ≈$0.0012",
            CloudText.engineRow(RouteInfo("cloud", provider = "groq-1", model = "whisper-large-v3-turbo", costUsd = 0.0012, costEstimated = true), "Groq"))
        val u = CloudUsage("2026-10", listOf(CloudUsage.ProviderUsage("groq-1", 3, 90_000, 0.004, true)))
        val lines = CloudText.usageLines(u, CloudSettings("groq-1", listOf(groq), monthlyBudgetUsd = 0.003))
        assertEquals("Month 2026-10", lines[0])
        assertEquals("Groq: 3 dictations · 1.5 min · ≈ $0.0040 (estimated)", lines[1])
        assertTrue(lines[2], lines[2].contains("reached"))
        assertEquals("no key", CloudText.mask(null))
        assertEquals("••••", CloudText.mask("abc"))
    }
}
