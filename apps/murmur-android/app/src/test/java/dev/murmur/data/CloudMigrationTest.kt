package dev.murmur.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * D28 one-time migration on the shell side, for every old preset: the router's own fixtures
 * (`crates/murmur-router/fixtures/migration/<case>.{v1,v2,report}.json`) stand in for
 * `nativeMigrateCloud`; this checks what only the app owns: Keystore keys, usage, prefs, notice.
 */
class CloudMigrationTest {
    private fun fixture(name: String) =
        requireNotNull(javaClass.classLoader!!.getResource("migration/$name")) { "fixture migration/$name not on the test classpath" }.readText()

    private val cases = listOf("assemblyai", "custom", "deepgram", "elevenlabs", "groq", "mixed_groq_active", "openai", "openrouter")

    private class Env {
        val kv = MemKV()
        val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("murmur_dev_token", fake))
    }

    /** What a D18 build left in the `dev` prefs for [v1]: the raw `cloud`, toggles, order, a key per provider, usage. */
    private fun legacyEnv(v1: JSONObject): Env {
        val e = Env()
        val ids = v1.getJSONObject("cloud").getJSONArray("providers").let { a -> (0 until a.length()).map { a.getJSONObject(it).getString("id") } }
        e.kv.edit {
            putString("cloud", v1.getJSONObject("cloud").toString())
            putBoolean("allow_cloud", v1.optBoolean("allow_cloud"))
            putString("route_order", v1.getJSONArray("order").let { a -> (0 until a.length()).joinToString(",") { a.getString(it) } })
            putString("server.host", "100.64.0.10")
        }
        for (id in ids) e.keys.set(id, "sk-LEAKCHECK-$id-0123456789")
        e.store.cloudUsageJson = CloudUsage(
            "2026-10", ids.mapIndexed { i, id -> CloudUsage.ProviderUsage(id, requests = 2L + i, billedAudioMs = 60_000, costUsd = 0.01 * (i + 1)) },
        ).toJson().toString()
        return e
    }

    @Test fun everyOldPresetMigratesKeysUsageAndSettings() {
        for (c in cases) {
            val v1 = JSONObject(fixture("$c.v1.json"))
            val v2 = JSONObject(fixture("$c.v2.json"))
            val report = JSONObject(fixture("$c.report.json"))
            val e = legacyEnv(v1)
            val usageBefore = CloudUsage.decode(e.store.cloudUsageJson).totalCostUsd
            assertNotNull(c, e.store.legacyCloudJson())
            var sent: String? = null
            val r = CloudMigration.run(e.store, e.keys) { json ->
                sent = json
                JSONObject().put("config", v2).put("report", report).toString()
            }
            assertNotNull(c, r)
            r!!
            // What the router got: the stored settings with the raw legacy providers (never a key).
            val dev = JSONObject(sent!!)
            assertEquals(c, v1.getJSONObject("cloud").getJSONArray("providers").length(), dev.getJSONObject("cloud").getJSONArray("providers").length())
            assertEquals(c, "100.64.0.10", dev.getJSONObject("server").getString("host"))
            assertFalse(c, sent!!.contains("LEAKCHECK"))

            // Keys: the kept OpenRouter entry's key moved to `openrouter`; every other vault destroyed.
            val oldIds = v1.getJSONObject("cloud").getJSONArray("providers").let { a -> (0 until a.length()).map { a.getJSONObject(it).getString("id") } }
            if (r.keyFrom != null) assertEquals(c, "sk-LEAKCHECK-${r.keyFrom}-0123456789", e.keys.get(OpenRouter.ID))
            else assertNull(c, e.keys.get(OpenRouter.ID))
            for (id in oldIds) {
                assertFalse("$c: $id ciphertext", e.keys.has(id))
                assertFalse("$c: $id Keystore key", SecretVault.cloudAlias(id) in e.fake.keys)
            }
            assertEquals(c, if (r.keyFrom != null) setOf("murmur_cloud_openrouter") else emptySet<String>(), e.fake.keys.keys.filter { it.startsWith("murmur_cloud_") }.toSet())

            // Settings: schema 2 as the router wrote it; another active provider switches cloud STT off.
            val s = e.store.load()
            assertEquals(c, CloudSettings.fromJson(v2.getJSONObject("cloud")), s.cloud)
            assertEquals(c, v2.optBoolean("allow_cloud", false), s.allowCloud)
            assertEquals(c, r.cloudDisabled, DevSettings.Route.CLOUD !in s.routeOrder)
            assertFalse(c, s.allowCloudCleanup)
            assertNull(c, e.store.legacyCloudJson())

            // Usage: same month total (budget stays honest); old ids renamed / folded into "removed".
            val u = CloudUsage.decode(e.store.cloudUsageJson)
            assertEquals(c, usageBefore, u.totalCostUsd, 1e-12)
            assertTrue(c, u.providers.all { it.providerId == OpenRouter.ID || it.providerId == CloudUsage.REMOVED_ID })

            // Notice: kept for the one-time banner.
            assertEquals(c, report.optString("notice").ifEmpty { null }, e.store.cloudNotice)

            // Idempotent: nothing left to migrate.
            assertNull(c, CloudMigration.run(e.store, e.keys) { error("must not be called again") })
        }
    }

    @Test fun mixedCaseKeepsTheOpenRouterKeyButTurnsCloudOff() {
        val e = legacyEnv(JSONObject(fixture("mixed_groq_active.v1.json")))
        val r = CloudMigration.run(e.store, e.keys) {
            JSONObject().put("config", JSONObject(fixture("mixed_groq_active.v2.json"))).put("report", JSONObject(fixture("mixed_groq_active.report.json"))).toString()
        }!!
        assertEquals("openrouter-1", r.keyFrom); assertEquals(listOf("groq-1"), r.deleteKeys); assertTrue(r.cloudDisabled)
        assertEquals("openai/gpt-transcribe", r.modelFrom); assertEquals(OpenRouter.DEFAULT_STT_MODEL, r.modelTo)
        assertEquals("sk-LEAKCHECK-openrouter-1-0123456789", e.keys.get(OpenRouter.ID))
        val s = e.store.load()
        assertFalse(s.allowCloud)
        assertEquals(listOf(DevSettings.Route.SERVER, DevSettings.Route.CLIENT), s.routeOrder)
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, s.cloud.stt.model)
        assertTrue(e.store.cloudNotice!!.startsWith("Cloud speech-to-text now uses OpenRouter only. Groq was removed"))
        val u = CloudUsage.decode(e.store.cloudUsageJson)
        assertEquals(listOf(OpenRouter.ID, CloudUsage.REMOVED_ID), u.providers.map { it.providerId })
    }

    @Test fun aMissingLibraryChangesNothing() {
        val e = legacyEnv(JSONObject(fixture("groq.v1.json")))
        val before = HashMap(e.kv.m)
        assertNull(CloudMigration.run(e.store, e.keys) { null })
        for (k in listOf("cloud", "allow_cloud", "route_order", DevStore.USAGE, CloudKeyStore.PREFIX + "groq-1")) assertEquals(k, before[k], e.kv.m[k])
        assertNotNull(e.store.legacyCloudJson())
        assertEquals("sk-LEAKCHECK-groq-1-0123456789", e.keys.get("groq-1"))
    }

    @Test fun nothingToDoOnASchema2Store() {
        val e = Env()
        assertNull(CloudMigration.run(e.store, e.keys) { error("not legacy") })
        e.store.saveCloud(cloud = CloudSettings())
        assertNull(CloudMigration.run(e.store, e.keys) { error("not legacy") })
    }

    @Test fun usageMirrorsTheRouter() {
        val r = CloudMigration.Report(migrated = true, keyFrom = "openrouter-1", removedIds = listOf("groq-1", "deepgram-1"))
        val before = CloudUsage("2026-10", listOf(
            CloudUsage.ProviderUsage("openrouter-1", 1, 1000, 0.1),
            CloudUsage.ProviderUsage("groq-1", 2, 2000, 0.2, costEstimated = true),
            CloudUsage.ProviderUsage("deepgram-1", 3, 3000, 0.3),
        ))
        val after = CloudUsage.decode(CloudMigration.migrateUsage(before.toJson().toString(), r))
        assertEquals(listOf(CloudUsage.ProviderUsage(OpenRouter.ID, 1, 1000, 0.1), CloudUsage.ProviderUsage(CloudUsage.REMOVED_ID, 5, 5000, 0.5, true)).map { it.providerId to it.requests },
            after.providers.map { it.providerId to it.requests })
        assertEquals(0.6, after.totalCostUsd, 1e-12)
        assertTrue(after.providers.last().costEstimated)
        assertNull(CloudMigration.migrateUsage(null, r))
    }
}
