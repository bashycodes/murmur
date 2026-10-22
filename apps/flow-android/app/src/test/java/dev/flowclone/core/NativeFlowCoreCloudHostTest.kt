package dev.flowclone.core

import com.sun.net.httpserver.HttpServer
import dev.flowclone.data.BuildCaps
import dev.flowclone.data.CloudPresets
import dev.flowclone.data.CloudSettings
import dev.flowclone.data.CloudText
import dev.flowclone.data.CloudUsage
import dev.flowclone.data.DevSettings
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.EngineRoute
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.net.InetSocketAddress
import java.util.Collections

/**
 * D18 end to end without a phone: the shipped Kotlin path (DevSettings JSON → EngineRoute →
 * nativeOpenRouter → NativeFlowCore cloud natives) against a host build of the REAL flow-ffi with
 * feature `cloud` (tools/probes/host_cloud/run.sh), talking to a JVM mock of an OpenAI-compatible
 * provider (Groq preset). Skips unless `-Pflow.jniLibDir` points at such a library.
 */
class NativeFlowCoreCloudHostTest {
    private lateinit var server: HttpServer
    private val auth = Collections.synchronizedList(mutableListOf<String?>())
    @Volatile private var status = 200

    private val key = "sk-LEAKCHECK-kotlin-0123456789abcd"

    @Before fun startProvider() {
        server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/openai/v1/audio/transcriptions") { ex ->
            ex.requestBody.readBytes()
            auth += ex.requestHeaders.getFirst("Authorization")
            val body = (if (status == 200) """{"text":"cloud words btw"}""" else """{"error":{"message":"bad key","code":"invalid_api_key"}}""").toByteArray()
            ex.responseHeaders.add("Content-Type", "application/json")
            ex.sendResponseHeaders(status, body.size.toLong())
            ex.responseBody.use { it.write(body) }
        }
        server.start()
    }

    @After fun stopProvider() = server.stop(0)

    private fun requireCloudLib() {
        assumeTrue("host flow-ffi not built", System.getProperty("flow.jniReal") == "1")
        assumeTrue("host flow-ffi built without feature cloud", NativeFlowCore.cloudCapabilities().cloud)
    }

    private fun dictate(r: NativeFlowCore): CommitResult {
        r.startSession(SessionConfig(languages = listOf("en"), dictionary = listOf(DictEntry("btw", "by the way"))))
        r.pushAudio(sine(24_000), 24_000)
        return r.commit()
    }

    @Test fun cloudRouteThroughTheShellsAssembly() {
        requireCloudLib()
        val provider = CloudPresets.of("groq").newProvider(emptyList())
            .copy(baseUrl = "http://127.0.0.1:${server.address.port}/openai/v1").withConsent(1)
        val dev = DevSettings(
            allowServer = false, allowClient = true, allowCloud = true,
            routeOrder = DevSettings.withCloudFirst(DevSettings.DEFAULT_ORDER),
            cloud = CloudSettings(active = provider.id, providers = listOf(provider)),
        )
        val caps = BuildCaps(local = true, remote = true, cloud = true)
        val plan = EngineRoute.plan(
            EnginePlan.Plan.Native(ModelConfig(asrPath = "", threads = 2), EnginePlan.LlmStatus.Off), dev, caps,
        ) as EngineRoute.Plan.Routed
        assertTrue(plan.routerConfigJson, plan.routerConfigJson.contains("\"order\":[\"cloud\",\"server\",\"client\"]"))
        assertFalse(plan.routerConfigJson.contains("LEAKCHECK"))
        val loader = FlowCoreLoader { NativeFlowCore.loadModels(it.copy(mockText = "local words btw")) }
        val a = EngineRoute.assemble(plan, loader, NativeFlowCore.routerOpener)
        assertTrue(a.description, a.description.startsWith("cloud: Groq"))
        val r = a.core as NativeFlowCore

        // No key yet: the cloud route is skipped, the device answers.
        val noKey = dictate(r)
        assertEquals("local", noKey.route!!.engine); assertEquals("cloud_not_configured", noKey.route!!.fallback)
        assertEquals("Cloud STT failed (set a key and accept the consent); used on-device", CloudText.fallbackMessage(noKey.route, dev.cloudFirst))

        // EngineHolder.syncCloud: the key reaches the router only through setCloudKey.
        r.setCloudKey(provider.id, key)
        val cloud = dictate(r)
        assertEquals(cloud.toString(), "cloud", cloud.route!!.engine)
        assertEquals("groq-1", cloud.route!!.provider)
        assertEquals("cloud words by the way ", cloud.finalText)
        assertEquals("C", DevText.bubbleBadge(cloud.route, showEngineBadge = false))
        assertTrue(DevText.lastDictation(cloud) { "Groq" }.startsWith("cloud · Groq · whisper-large-v3-turbo · ≈$"))
        assertEquals("Bearer $key", auth.last())
        val usage = CloudUsage.decode(r.cloudUsageJson())
        assertEquals(1L, usage.of("groq-1")!!.requests)
        assertTrue(usage.of("groq-1")!!.costEstimated)

        // Test provider: via the router (key passed) and via a temporary router.
        val ok = r.probeCloud(provider.toJson().toString(), key)
        assertTrue(DevText.cloudProbe(ok), ok.ok); assertEquals("cloud words btw", ok.transcript)
        assertTrue(NativeFlowCore.probeCloud(provider.toJson().toString(), key).ok)
        assertEquals("cloud_not_configured", NativeFlowCore.probeCloud(provider.toJson().toString(), null).error)

        // Provider rejects the key: auth_failed, same audio on the device, user message, cloud circuit open.
        status = 401
        val fb = dictate(r)
        assertEquals("local", fb.route!!.engine); assertEquals("auth_failed", fb.route!!.fallback)
        assertEquals("local words by the way ", fb.finalText)
        assertEquals(cloud.timings.audioMs, fb.timings.audioMs)
        assertEquals("Cloud STT failed (key rejected); used on-device", CloudText.fallbackMessage(fb.route, dev.cloudFirst))
        val st = r.routeStatus()
        assertTrue(st.cloudCircuit!!.open); assertEquals("auth_failed", st.cloudCircuit!!.reason)
        assertEquals("auth_failed: key rejected", DevText.cloudProbe(r.probeCloud(provider.toJson().toString(), null)).substringBefore(" — "))

        // Usage persistence round trip (EngineHolder.persistCloudUsage → syncCloud on the next open).
        val saved = r.cloudUsageJson()!!
        r.resetCloudUsage()
        assertTrue(CloudUsage.decode(r.cloudUsageJson()).providers.isEmpty())
        assertTrue(r.setCloudUsage(saved))
        assertEquals(1L, CloudUsage.decode(r.cloudUsageJson()).of("groq-1")!!.requests)
        assertFalse(r.setCloudUsage("not json"))

        // Forgetting the key (provider deleted) takes the cloud out again.
        status = 200
        r.updateRouter(dev.routerConfigFor(caps))
        r.setCloudKey(provider.id, null)
        assertEquals("local", dictate(r).route!!.engine)
        r.unload()
    }

    private fun sine(n: Int) = ShortArray(n) { i -> (Math.sin(i * 0.05) * 8000).toInt().toShort() }
}
