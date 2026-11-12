package dev.murmur.core

import dev.murmur.data.BuildCaps
import dev.murmur.data.CloudCatalog
import dev.murmur.data.CloudKeyStore
import dev.murmur.data.CloudMigration
import dev.murmur.data.CloudSettings
import dev.murmur.data.DevSettings
import dev.murmur.data.DevStore
import dev.murmur.data.EnginePlan
import dev.murmur.data.EngineRoute
import dev.murmur.data.OpenRouter
import dev.murmur.data.CloudPage
import dev.murmur.data.SecretVault
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * D28 end to end without a phone or the network: the shipped Kotlin path (DevSettings JSON →
 * EngineRoute → nativeOpenRouter → the OpenRouter natives) against a host build of the REAL
 * murmur-ffi with feature `cloud` (tools/probes/host_cloud/run.sh). The base URL is fixed in
 * murmur-cloud, so nothing here may reach a server: no key is ever set, so the cloud route and
 * cloud cleanup are refused up front. Skips unless `-Pmurmur.jniLibDir` points at such a library.
 */
class NativeMurmurCoreCloudHostTest {
    private fun requireCloudLib() {
        assumeTrue("host murmur-ffi not built", System.getProperty("murmur.jniReal") == "1")
        assumeTrue("host murmur-ffi built without feature cloud", NativeMurmurCore.cloudCapabilities().cloud)
    }

    private fun fixture(name: String) = requireNotNull(javaClass.classLoader!!.getResource(name)).readText()

    @Test fun nativeMigrationMatchesTheRouterFixtures() {
        requireCloudLib()
        for (c in listOf("groq", "openrouter", "mixed_groq_active")) {
            val out = NativeMurmurCore.migrateCloud(fixture("migration/$c.v1.json"))
            val (cfg, r) = CloudMigration.parse(out)!!
            val want = CloudMigration.decodeReport(JSONObject(fixture("migration/$c.report.json")))
            assertEquals(c, want, r)
            assertEquals(c, CloudSettings.fromJson(JSONObject(fixture("migration/$c.v2.json")).getJSONObject("cloud")), CloudSettings.fromJson(cfg.getJSONObject("cloud")))
        }
        // The full shell path through the real native: keys moved, prefs schema 2.
        val kv = dev.murmur.data.MemKV(); val fake = dev.murmur.data.FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("t", fake))
        val v1 = JSONObject(fixture("migration/mixed_groq_active.v1.json"))
        kv.edit { putString("cloud", v1.getJSONObject("cloud").toString()); putBoolean("allow_cloud", true); putString("route_order", "cloud,server,client") }
        keys.set("openrouter-1", "sk-or-LEAKCHECK-0123456789"); keys.set("groq-1", "gsk-LEAKCHECK-0123456789")
        val r = CloudMigration.run(store, keys) { NativeMurmurCore.migrateCloud(it) }!!
        assertTrue(r.cloudDisabled)
        assertEquals("sk-or-LEAKCHECK-0123456789", keys.get(OpenRouter.ID)); assertFalse(keys.has("groq-1"))
        assertFalse(store.load().allowCloud)
        assertNotNull(store.cloudNotice)
    }

    @Test fun catalogueIsServedOfflineFromTheBakedList() {
        requireCloudLib()
        val stt = CloudCatalog.decodeRows(NativeMurmurCore.cloudCatalog("stt", "", CloudCatalog.Filters().toJson()))
        assertTrue(stt.size.toString(), stt.size >= 5)
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, stt.first().id)
        assertTrue(stt.first().recommended && stt.first().zdr)
        assertTrue(stt.all { it.zdr })
        val chat = CloudCatalog.decodeRows(NativeMurmurCore.cloudCatalog("stt", "", CloudCatalog.Filters(zdrOnly = false, sttPath = "chat_audio").toJson()))
        assertTrue(chat.all { it.chatAudio })
        val cleanup = CloudCatalog.decodeRows(NativeMurmurCore.cloudCatalog("cleanup", "llama", CloudCatalog.Filters().toJson()))
        assertEquals(OpenRouter.DEFAULT_CLEANUP_MODEL, cleanup.first().id)
        assertTrue(cleanup.first().priceLabel, cleanup.first().priceLabel.endsWith("per dictation"))
        assertEquals("[]", NativeMurmurCore.cloudCatalog("nope", "", "{}"))
    }

    @Test fun routerWithBothUsesButNoKeyRefusesUpFront() {
        requireCloudLib()
        val caps = BuildCaps(local = true, remote = true, cloud = true)
        val dev = CloudPage.enableCleanup(
            CloudPage.enableStt(DevSettings(allowServer = false, allowClient = true), 1), 1,
        ).copy(cleanupOrder = listOf(DevSettings.CleanupRoute.CLOUD, DevSettings.CleanupRoute.LOCAL, DevSettings.CleanupRoute.SERVER))
        val plan = EngineRoute.plan(EnginePlan.Plan.Native(ModelConfig(asrPath = "", threads = 2), EnginePlan.LlmStatus.Off), dev, caps) as EngineRoute.Plan.Routed
        assertTrue(plan.routerConfigJson, plan.routerConfigJson.contains("\"cleanup_order\":[\"cloud\",\"local\",\"server\"]"))
        val loader = MurmurCoreLoader { NativeMurmurCore.loadModels(it.copy(mockText = "local words btw")) }
        val a = EngineRoute.assemble(plan, loader, NativeMurmurCore.routerOpener)
        assertTrue(a.description, a.description.startsWith("cloud: OpenRouter"))
        val r = a.core as NativeMurmurCore

        r.startSession(SessionConfig(languages = listOf("en"), dictionary = listOf(DictEntry("btw", "by the way"))))
        r.pushAudio(ShortArray(24_000) { i -> (Math.sin(i * 0.05) * 8000).toInt().toShort() }, 24_000)
        val res = r.commit()
        assertEquals("local", res.route!!.engine); assertEquals("cloud_not_configured", res.route!!.fallback)

        val st = r.cleanupStatus()
        assertTrue(st.supported); assertTrue(st.enabled); assertTrue(st.consent)
        assertEquals("cloud_cleanup_not_configured", st.blocked)
        assertEquals(listOf("cloud", "local", "server"), st.order)
        assertEquals(CloudKeyInfo(error = "cloud_not_configured"), r.cloudKeyInfo())
        assertTrue(r.setCloudModel("cleanup", "meta-llama/llama-3.1-8b-instruct:nitro")!!)
        assertFalse(r.setCloudModel("cleanup", "Not An Id")!!)
        assertFalse(r.setCloudModel("tts", "a/b")!!)
        assertEquals(null, r.cloudNotice())
        assertTrue(r.setCleanup("""{"allow_cloud_cleanup":false}"""))
        assertFalse(r.cleanupStatus().enabled)
        assertFalse(r.setCleanup("""{"cleanup":{"timeout_ms":100}}"""))
        r.unload()
    }
}
