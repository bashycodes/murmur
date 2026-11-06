package dev.flowclone

import dev.flowclone.core.CoreException
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FlowCore
import dev.flowclone.core.FlowCoreLoader
import dev.flowclone.core.ModelConfig
import dev.flowclone.core.AsrEngine
import dev.flowclone.data.LoadSet
import dev.flowclone.data.LoadedAsr
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * D37 language switch while a model loads: the engine builds run through [EngineHolder]'s test
 * seams (a fake core per route, each held at a latch), so the real load sequencing is exercised.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class EngineHolderTest {
    private val app get() = FlowApp.of(RuntimeEnvironment.getApplication())

    @Volatile private var want: LoadSet = LoadSet.Language("he")
    private val gates = ConcurrentHashMap<String, CountDownLatch>()
    private val built = CopyOnWriteArrayList<String>()
    private val cores = ConcurrentHashMap<String, FakeFlowCore>()
    /** Per build: was every earlier engine already let go of when it started (one model in RAM)? */
    private val othersFreedAtStart = ConcurrentHashMap<String, Boolean>()

    private fun key(set: LoadSet) = when (set) {
        LoadSet.Default -> "default"
        is LoadSet.Language -> set.code
        LoadSet.Both -> "both"
    }

    /** Load 3: `releaseKeptLlm` calls. */
    private val releases = java.util.concurrent.atomic.AtomicInteger()
    private val loader = object : FlowCoreLoader {
        override fun loadModels(cfg: ModelConfig): FlowCore = error("the builder seam loads")
        override fun releaseKeptLlm() { releases.incrementAndGet() }
    }

    /** Load 3: every `keepLlm` call, as (engine, was it still loaded). */
    private fun keeps() = cores.flatMap { (k, c) -> c.keepLlmCalls.map { k to it } }

    /** The next build of [k] fails (the speech model did not load). */
    private val failing = ConcurrentHashMap.newKeySet<String>()

    private fun holder() = EngineHolder(app, loader = loader, setProbe = { want }, builder = { set ->
        val k = key(set)
        othersFreedAtStart[k] = cores.values.all { it.unloaded }
        built += k
        gates[k]?.await(5, TimeUnit.SECONDS)
        if (failing.remove(k)) EngineHolder.State.Failed("no $k model", "model_not_found")
        else {
            val c = FakeFlowCore(k, hasLlm = true)
            cores[k] = c
            EngineHolder.State.Ready(c, "fake $k", asr = LoadedAsr(AsrEngine.WHISPER, "$k.bin"))
        }
    })

    private fun waitFor(what: String, cond: () -> Boolean) {
        val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (!cond()) {
            if (System.nanoTime() > until) fail("timed out waiting for $what")
            Thread.sleep(5)
        }
    }

    private fun loadingMessage(h: EngineHolder): String {
        try { h.current() } catch (e: CoreException) {
            assertEquals(CoreException.KIND_LOADING, e.kind)
            return e.message!!
        }
        fail("engine is not loading"); throw AssertionError()
    }

    @Test fun aLoadingEngineSaysWhichModelWithTheLoadingKind() {
        gates["he"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        assertEquals(EngineHolder.HEBREW_LOADING, loadingMessage(h))
        gates["he"]!!.countDown()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
    }

    @Test fun switchingToEnglishWhileTheHebrewModelLoadsLoadsTheDefaultModelNext() {
        gates["he"] = CountDownLatch(1)
        gates["default"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }

        want = LoadSet.Default // the keyboard switched to English
        h.ensureLanguage()
        assertEquals("names the model the dictation now needs", EngineHolder.DEFAULT_LOADING, loadingMessage(h))

        gates["he"]!!.countDown()
        waitFor("default build") { "default" in built }
        assertTrue("the Hebrew engine was let go of before the default one loads", othersFreedAtStart["default"]!!)
        assertTrue("still loading, never Ready with the Hebrew model", h.state.value is EngineHolder.State.Loading)
        assertEquals(EngineHolder.DEFAULT_LOADING, loadingMessage(h))

        gates["default"]!!.countDown()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        assertEquals("fake default", (h.state.value as EngineHolder.State.Ready).description)
        assertEquals(listOf("he", "default"), built.toList())
    }

    @Test fun switchingBackBeforeTheLoadEndsKeepsTheModelLoading() {
        gates["he"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        want = LoadSet.Default; h.ensureLanguage()
        want = LoadSet.Language("he"); h.ensureLanguage()
        assertEquals(EngineHolder.HEBREW_LOADING, loadingMessage(h))
        gates["he"]!!.countDown()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        assertEquals("fake he", (h.state.value as EngineHolder.State.Ready).description)
        assertEquals(listOf("he"), built.toList())
    }

    @Test fun withBothLoadedAKeyboardSwitchNeverReloadsAndTogglingReloadsOnce() {
        want = LoadSet.Both
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        // A switch to English and back: the wanted set stays both, so nothing reloads.
        repeat(3) { h.ensureLanguage() }
        assertEquals(listOf("both"), built.toList())
        assertTrue(h.state.value is EngineHolder.State.Ready)
        // "Keep both languages loaded" turned off with the keyboard on Hebrew: one reload, to the Hebrew model.
        want = LoadSet.Language("he")
        h.ensureLanguage()
        waitFor("hebrew build") { (h.state.value as? EngineHolder.State.Ready)?.description == "fake he" }
        h.ensureLanguage()
        assertEquals(listOf("both", "he"), built.toList())
    }

    @Test fun aLowMemoryUnloadReloadsBothModelsAtTheNextLoad() {
        want = LoadSet.Both
        gates["both"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("both build") { "both" in built }
        assertEquals(EngineHolder.BOTH_LOADING, loadingMessage(h))
        gates["both"]!!.countDown()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        h.unload() // FlowService.onTrimMemory
        assertTrue(cores["both"]!!.unloaded)
        h.ensureLoaded()
        waitFor("ready again") { h.state.value is EngineHolder.State.Ready }
        assertEquals(listOf("both", "both"), built.toList())
    }

    @Test fun aSwitchFromTheReadyHebrewModelNamesTheDefaultModel() {
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        gates["default"] = CountDownLatch(1)
        want = LoadSet.Default
        h.ensureLanguage()
        waitFor("default build") { "default" in built }
        assertEquals(EngineHolder.DEFAULT_LOADING, loadingMessage(h))
        gates["default"]!!.countDown()
        waitFor("ready") { (h.state.value as? EngineHolder.State.Ready)?.description == "fake default" }
    }

    @Test fun aLanguageSwitchKeepsTheLlmOfTheEngineItFrees() {
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        assertEquals("a first load keeps nothing", emptyList<Pair<String, Boolean>>(), keeps())
        want = LoadSet.Default
        h.ensureLanguage()
        waitFor("default ready") { (h.state.value as? EngineHolder.State.Ready)?.description == "fake default" }
        // The Hebrew engine's own LLM, kept while it was still loaded; what the load did not take is freed after it.
        assertEquals(listOf("he" to true), keeps())
        assertEquals(1, releases.get())
        assertTrue(cores["he"]!!.unloaded)
    }

    @Test fun aFailedLoadLeavesTheKeptLlmForTheRetry() {
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        failing += "default"
        want = LoadSet.Default
        h.ensureLanguage()
        waitFor("failed") { h.state.value is EngineHolder.State.Failed }
        assertEquals("still kept for the retry", 0, releases.get())
        h.ensureLoaded() // retry
        waitFor("default ready") { (h.state.value as? EngineHolder.State.Ready)?.description == "fake default" }
        assertEquals(1, releases.get())
    }

    @Test fun aLowMemoryUnloadFreesAKeptLlm() {
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        failing += "default"
        want = LoadSet.Default
        h.ensureLanguage()
        waitFor("failed") { h.state.value is EngineHolder.State.Failed }
        h.unload() // FlowService.onTrimMemory
        waitFor("released") { releases.get() == 1 }
    }

    @Test fun aLowMemoryUnloadOrASettingsReloadKeepsNoLlm() {
        val h = holder()
        h.ensureLoaded()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        h.unload() // FlowService.onTrimMemory
        h.ensureLoaded()
        waitFor("ready again") { h.state.value is EngineHolder.State.Ready }
        h.reload() // a model or developer setting changed
        waitFor("reloaded") { built.size == 3 && h.state.value is EngineHolder.State.Ready }
        assertEquals(emptyList<Pair<String, Boolean>>(), keeps())
        assertEquals(0, releases.get())
    }

    // ---- D55: a dictation records while the engine loads ----

    /** Runs [LoadWait.await] off the test thread, so the builds' latches can be released meanwhile. */
    private fun startWaiting(w: dev.flowclone.session.LoadWait) =
        kotlinx.coroutines.CoroutineScope(Dispatchers.Default).async { withTimeout(5_000) { w.await() } }

    /** A holder whose one build returns [ready] once [gates]"x" opens. */
    private fun holderBuilding(ready: () -> EngineHolder.State) = EngineHolder(app, loader = loader, setProbe = { want }, builder = {
        built += "x"
        gates["x"]?.await(5, TimeUnit.SECONDS)
        ready()
    })

    @Test fun aWaitIsOfferedWhileLoadingAndRightAfterButNotForAFailedLoad() {
        val h = holder()
        gates["he"] = CountDownLatch(1)
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        if (!app.caps.local) {
            assertNull("remote flavour: no phone model, the start is refused as before", h.loadWait())
            gates["he"]!!.countDown()
            return
        }
        assertEquals(EngineHolder.HEBREW_LOADING, h.loadWait()!!.label)
        gates["he"]!!.countDown()
        waitFor("ready") { h.state.value is EngineHolder.State.Ready }
        // The load ended between the dictation's "still loading" and its wait: picked up at once.
        val got = runBlocking { h.loadWait()!!.await() }
        assertEquals(cores["he"], got.core)
        failing += "he"
        h.reload()
        waitFor("failed") { h.state.value is EngineHolder.State.Failed }
        assertNull("a failed load is refused as before", h.loadWait())
    }

    @Test fun aDictationWaitingForTheLoadGetsThePhoneEngineOnceLoaded() {
        assumeTrue(app.caps.local)
        gates["he"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        val got = startWaiting(h.loadWait()!!)
        Thread.sleep(50)
        assertTrue("waits for the load", got.isActive)
        gates["he"]!!.countDown()
        val engine = runBlocking { got.await() }
        assertEquals(cores["he"], engine.core)
        assertTrue("a phone model loaded: transcribed on the phone only", engine.phoneOnly)
    }

    @Test fun aLanguageSwitchDuringTheWaitHandsOverTheModelLoadedLast() {
        assumeTrue(app.caps.local)
        gates["he"] = CountDownLatch(1)
        gates["default"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        val got = startWaiting(h.loadWait()!!)
        want = LoadSet.Default
        h.ensureLanguage()
        gates["he"]!!.countDown()
        waitFor("default build") { "default" in built }
        Thread.sleep(50)
        assertTrue("never the dropped Hebrew engine", got.isActive)
        gates["default"]!!.countDown()
        val engine = runBlocking { got.await() }
        assertEquals(cores["default"], engine.core)
    }

    /**
     * D37 / D55: a load re-reads the keyboard when it ends, so a dictation that waits for the
     * default model while the keyboard switches to Hebrew is handed the Hebrew model (the
     * pre-D58 way a mid-recording switch changed the result; D58's stop read then agrees).
     */
    @Test fun keyboardSwitchedWhileARecordingWaitsGetsTheNewLanguagesModel() {
        assumeTrue(app.caps.local)
        want = LoadSet.Default
        gates["default"] = CountDownLatch(1)
        gates["he"] = CountDownLatch(1)
        val h = holder()
        h.ensureLoaded()
        waitFor("default build") { "default" in built }
        val got = startWaiting(h.loadWait()!!)
        want = LoadSet.Language("he") // the keyboard switched mid-recording
        gates["default"]!!.countDown()
        waitFor("hebrew build") { "he" in built }
        Thread.sleep(50)
        assertTrue("never the dropped default engine", got.isActive)
        gates["he"]!!.countDown()
        val engine = runBlocking { got.await() }
        assertEquals(cores["he"], engine.core)
    }

    @Test fun aFailedLoadFailsTheWait() {
        assumeTrue(app.caps.local)
        gates["he"] = CountDownLatch(1)
        failing += "he"
        val h = holder()
        h.ensureLoaded()
        waitFor("hebrew build") { "he" in built }
        val got = startWaiting(h.loadWait()!!)
        gates["he"]!!.countDown()
        val e = runBlocking { runCatching { got.await() }.exceptionOrNull() }
        assertEquals("no he model", (e as CoreException).message)
    }

    @Test fun aPlannedPhoneModelThatDidNotLoadFailsTheWaitInsteadOfUsingTheServer() {
        assumeTrue(app.caps.local)
        gates["x"] = CountDownLatch(1)
        // The router opened with the server only: the phone model the plan wanted failed to load.
        val h = holderBuilding { EngineHolder.State.Ready(FakeFlowCore("x"), "server", clientNote = "No speech model imported.", localPlanned = true) }
        h.ensureLoaded()
        val got = startWaiting(h.loadWait()!!)
        gates["x"]!!.countDown()
        val e = runBlocking { runCatching { got.await() }.exceptionOrNull() }
        assertEquals("The phone speech model did not load: No speech model imported.", e?.message)
    }

    @Test fun theOnDemandRouteWaitsForTheRouterAndStartsOnItsNormalRoute() {
        assumeTrue(app.caps.local)
        gates["x"] = CountDownLatch(1)
        // D24 keep warm off, server first: no phone model loads before the router opens.
        val core = FakeFlowCore("x")
        val h = holderBuilding {
            EngineHolder.State.Ready(core, "server → phone on demand", onDemandAsr = LoadedAsr(AsrEngine.WHISPER, "he.bin"))
        }
        h.ensureLoaded()
        val got = startWaiting(assertNotNullOf(h.loadWait()))
        gates["x"]!!.countDown()
        val engine = runBlocking { got.await() }
        assertEquals(core, engine.core)
        assertFalse("the server may take it", engine.phoneOnly)
    }

    @Test fun aWaitGivesUpAfterRepeatedUnloads() {
        assumeTrue(app.caps.local)
        // Every load ends unloaded (e.g. low memory): the wait restarts it a bounded number of times.
        val h = holderBuilding { EngineHolder.State.NotLoaded }
        gates["x"] = CountDownLatch(1)
        h.ensureLoaded()
        val got = startWaiting(h.loadWait()!!)
        gates["x"]!!.countDown()
        val e = runBlocking { runCatching { got.await() }.exceptionOrNull() }
        assertEquals("The speech model was unloaded while loading. Try again.", e?.message)
        assertEquals(1 + EngineHolder.MAX_LOAD_RESTARTS, built.size)
    }

    private fun <T : Any> assertNotNullOf(v: T?): T { assertNotNull(v); return v!! }
}
