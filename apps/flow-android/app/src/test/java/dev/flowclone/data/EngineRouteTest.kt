package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CoreException
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FlowCore
import dev.flowclone.core.ModelConfig
import dev.flowclone.core.ParakeetDecoding
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** D17 §6.2 Kotlin flow: which local engine to load, and what the router is opened with, per flavour × toggles. */
class EngineRouteTest {
    private val exists: (String) -> Boolean = { it.startsWith("/m/") }
    private fun local(s: Settings = Settings(), asr: String? = "/m/base.bin") = EnginePlan.plan(s, asr, null, exists, 4)
    private val server = DevSettings(allowServer = true, allowClient = true, host = "100.64.0.10", token = "0123456789abcdef")

    private class Opener : RouterOpener {
        var json: String? = null
        var local: FlowCore? = null
        var fail: CoreException? = null
        val router = FakeFlowCore("routed")
        override fun open(routerConfigJson: String, local: FlowCore?): FlowCore {
            json = routerConfigJson; this.local = local
            fail?.let { throw it }
            return router
        }
    }

    private fun routed(p: EngineRoute.Plan) = p as EngineRoute.Plan.Routed

    @Test fun mockEngineBypassesTheRouter() =
        assertEquals(EngineRoute.Plan.Mock, EngineRoute.plan(local(Settings(mockEngine = true)), server, BuildCaps.ALL))

    @Test fun bothOffIsUnavailableWithTheRouterKind() {
        val p = EngineRoute.plan(local(), DevSettings(allowServer = false, allowClient = false), BuildCaps.ALL) as EngineRoute.Plan.Unavailable
        assertEquals("no_engine_enabled", p.kind)
        assertTrue(p.message, p.message.startsWith("Dictation disabled"))
        // A remote build with the server toggle off is "both off" too.
        assertTrue(EngineRoute.plan(local(), DevSettings(allowServer = false, allowClient = true), BuildCaps.REMOTE) is EngineRoute.Plan.Unavailable)
    }

    @Test fun localFlavourLoadsLocalAndNeverAsksForAServer() {
        val p = routed(EngineRoute.plan(local(), server, BuildCaps.LOCAL))
        assertTrue(p.local != null)
        val j = JSONObject(p.routerConfigJson)
        assertFalse(j.getBoolean("allow_server"))
        assertTrue(j.getBoolean("allow_client"))
    }

    @Test fun localFlavourWithoutModelIsMissingAsBefore() {
        val p = EngineRoute.plan(local(asr = null), DevSettings.defaults(BuildCaps.LOCAL), BuildCaps.LOCAL) as EngineRoute.Plan.Unavailable
        assertEquals("model_not_found", p.kind)
    }

    @Test fun remoteFlavourNeverLoadsLocalModels() {
        val p = routed(EngineRoute.plan(EnginePlan.Plan.Missing("no model"), DevSettings.defaults(BuildCaps.REMOTE).copy(host = "10.0.0.2"), BuildCaps.REMOTE))
        assertNull(p.local)
        assertNull("a remote build has no client to explain", p.localNote)
        assertFalse(JSONObject(p.routerConfigJson).getBoolean("allow_client"))
    }

    @Test fun hybridKeepsLocalWarmEvenWhenServerFirst() {
        assertTrue(routed(EngineRoute.plan(local(), server, BuildCaps.ALL)).local != null)
        assertNull(routed(EngineRoute.plan(local(), server.copy(keepLocalWarm = false), BuildCaps.ALL)).local)
        assertTrue("client first always loads", routed(EngineRoute.plan(local(), server.copy(keepLocalWarm = false, routeOrder = DevSettings.orderFor(DevSettings.Prefer.CLIENT)), BuildCaps.ALL)).local != null)
    }

    @Test fun hybridWithoutModelStillServesTheServerRoute() {
        val p = routed(EngineRoute.plan(local(asr = null), server, BuildCaps.ALL))
        assertNull(p.local)
        assertTrue(p.localNote!!.contains("No speech model"))
    }

    @Test fun assembleHandsTheLocalCoreToTheRouter() {
        val o = Opener()
        val loaded = FakeFlowCore("local")
        val r = EngineRoute.assemble(routed(EngineRoute.plan(local(), server, BuildCaps.ALL)), { loaded }, o)
        assertSame(o.router, r.core)
        assertSame(loaded, o.local)
        assertEquals("server: 100.64.0.10:7313 → client: base.bin", r.description)
        assertEquals(LoadedAsr(AsrEngine.WHISPER, "base.bin"), r.asr)
        assertTrue(JSONObject(o.json!!).getJSONObject("server").getString("token").isNotEmpty())
    }

    @Test fun assembleOpensServerOnlyWhenLocalFails() {
        val o = Opener()
        val r = EngineRoute.assemble(routed(EngineRoute.plan(local(), server, BuildCaps.ALL)), { throw CoreException("model not found: /m/base.bin") }, o)
        assertSame(o.router, r.core)
        assertNull(o.local)
        assertTrue(r.description, r.description.contains("client unavailable: model not found"))
    }

    @Test fun clientOnlyLocalFailureIsAnError() {
        val o = Opener()
        val r = EngineRoute.assemble(routed(EngineRoute.plan(local(), DevSettings.defaults(BuildCaps.LOCAL), BuildCaps.LOCAL)), { throw CoreException("bad model") }, o)
        assertNull(r.core)
        assertNull("router never opened", o.json)
        assertEquals("bad model", r.error)
    }

    @Test fun routerOpenFailureCarriesItsKind() {
        val o = Opener().apply { fail = CoreException("bad_config: public address: TLS (wss) is required", "bad_config") }
        val r = EngineRoute.assemble(routed(EngineRoute.plan(local(), server, BuildCaps.ALL)), { FakeFlowCore() }, o)
        assertNull(r.core)
        assertEquals("bad_config", r.errorKind)
    }

    // ---- Parakeet selection ----

    @Test fun parakeetSelectedAndPresentWithWhisperFallback() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/m/pk", isModelDir = { it == "/m/pk" }) as EnginePlan.Plan.Native
        assertEquals(AsrEngine.PARAKEET, p.config.asrEngine)
        assertEquals("/m/pk", p.config.asrPath)
        assertEquals("/m/base.bin", p.whisperFallback!!.asrPath)
    }

    @Test fun parakeetSelectedButMissingUsesWhisper() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = null) as EnginePlan.Plan.Native
        assertEquals(AsrEngine.WHISPER, p.config.asrEngine)
    }

    @Test fun parakeetOnlyNoWhisperIsFine() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), null, null, exists, 4, parakeetPath = "/m/pk", isModelDir = { true }) as EnginePlan.Plan.Native
        assertNull(p.whisperFallback)
    }

    @Test fun parakeetLoadFailureFallsBackToWhisper() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/m/pk", isModelDir = { true }) as EnginePlan.Plan.Native
        val tried = mutableListOf<ModelConfig>()
        val r = EnginePlan.load(p) { cfg -> tried += cfg; if (cfg.asrEngine == AsrEngine.PARAKEET) throw CoreException("model load failed: oom") else FakeFlowCore() }
        assertTrue(r.core != null)
        assertEquals(listOf(AsrEngine.PARAKEET, AsrEngine.WHISPER), tried.map { it.asrEngine })
        assertTrue(r.description, r.description.contains("Parakeet failed"))
        assertEquals("what actually loaded", LoadedAsr(AsrEngine.WHISPER, "base.bin"), r.asr)
    }

    @Test fun loadedAsrNamesTheParakeetFolder() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/m/pk-int8/", isModelDir = { true }) as EnginePlan.Plan.Native
        assertEquals(LoadedAsr(AsrEngine.PARAKEET, "pk-int8"), EnginePlan.load(p) { FakeFlowCore() }.asr)
    }

    @Test fun loadedAsrIsNullWhenNothingLoaded() {
        val p = local() as EnginePlan.Plan.Native
        assertNull(EnginePlan.load(p) { throw CoreException("bad") }.asr)
    }

    @Test fun parakeetFolderSelection() {
        val names = listOf("encoder.int8.onnx", "encoder.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt", "README.md", "test_wavs")
        assertEquals(
            mapOf("encoder" to "encoder.int8.onnx", "decoder" to "decoder.int8.onnx", "joiner" to "joiner.int8.onnx", "tokens" to "tokens.txt"),
            ParakeetFiles.select(names),
        )
        assertEquals("encoder.onnx", ParakeetFiles.select(listOf("encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"))!!["encoder"])
        assertNull(ParakeetFiles.select(listOf("encoder.int8.onnx", "tokens.txt")))
        assertEquals(listOf("decoder*.onnx", "joiner*.onnx"), ParakeetFiles.missing(listOf("encoder.int8.onnx", "tokens.txt")))
    }

    @Test fun spokenCleanupPrefReachesTheSessionConfig() {
        assertTrue(Settings().toSessionConfig(emptyList()).spokenCleanup)
        assertFalse(Settings(spokenCleanup = false).toSessionConfig(emptyList()).spokenCleanup)
    }

    // ---- D22: one Parakeet recognizer on the phone (saves ~0.6 GB RSS) ----

    @Test fun parakeetDecodesGreedyOnDeviceByDefault() {
        val p = EnginePlan.plan(Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/m/pk", isModelDir = { true }) as EnginePlan.Plan.Native
        assertEquals(ParakeetDecoding.GREEDY, p.config.parakeetDecoding)
        assertEquals("whisper ignores it; keep its JSON unchanged", ParakeetDecoding.AUTO, p.whisperFallback!!.parakeetDecoding)
    }

    @Test fun parakeetDecodingDevOverride() {
        val p = EnginePlan.plan(
            Settings(asrEngine = AsrEngine.PARAKEET), null, null, exists, 4, parakeetPath = "/m/pk", isModelDir = { true },
            parakeetDecoding = ParakeetDecoding.AUTO,
        ) as EnginePlan.Plan.Native
        assertEquals(ParakeetDecoding.AUTO, p.config.parakeetDecoding)
    }
}
