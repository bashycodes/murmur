package dev.flowclone.data

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.RouteInfo
import dev.flowclone.core.Timings
import dev.flowclone.data.DevPages.Page
import dev.flowclone.data.DevSettings.Route
import dev.flowclone.data.DevSettingsForm.Field
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Developer settings hub + sub-pages: per-page drafts, dirty / save gating, row subtitles. */
class DevPagesTest {
    private val caps = BuildCaps.ALL
    private val saved = DevSettings.defaults(caps).copy(host = "100.64.0.10", token = "tok_0123456789abcdef")

    // ---- ownership: every form field / input lives on exactly one page ----

    @Test fun everyFieldHasOnePage() {
        for (f in Field.entries) {
            val owners = Page.entries.filter { f in DevPages.fields(it) }
            assertEquals("$f", listOf(DevPages.pageOf(f)), owners)
        }
        assertEquals(Page.SERVER, DevPages.pageOf(Field.TOKEN))
    }

    // ---- drafts ----

    @Test fun freshDraftIsCleanAndCannotSave() {
        val d = DevDraft.of(saved, Page.SERVER)
        assertFalse(d.dirty)
        assertFalse("Save stays disabled until something changed", d.canSave(caps))
    }

    @Test fun editMakesDirtyAndUndoMakesClean() {
        val d = DevDraft.of(saved, Page.SERVER)
        val e = d.edit { copy(port = "7314") }
        assertTrue(e.dirty); assertTrue(e.canSave(caps))
        assertFalse(e.edit { copy(port = saved.port.toString()) }.dirty)
    }

    @Test fun invalidEditIsDirtyButCannotSave() {
        val e = DevDraft.of(saved, Page.SERVER).edit { copy(port = "0") }
        assertTrue(e.dirty)
        assertFalse(e.canSave(caps))
        assertTrue(Field.PORT in e.result(caps).errors)
    }

    @Test fun savedSettingsEqualOldWholePageSave() {
        // Same semantics as the single-page form: the whole DevSettings is validated + written.
        val e = DevDraft.of(saved, Page.SERVER).edit { copy(port = "7400", healthTtlS = "60") }
        val want = DevSettingsForm.validate(DevSettingsForm.Input.of(saved).copy(port = "7400", healthTtlS = "60"), caps).settings
        assertEquals(want, e.result(caps).settings)
        assertEquals(saved.token, e.result(caps).settings!!.token)
    }

    @Test fun rebaseKeepsThisPagesEditsAndTakesTheRestFromTheStore() {
        // Speech to text / the OpenRouter page saved meanwhile; Server keeps its own edit.
        val d = DevDraft.of(saved, Page.SERVER).edit { copy(port = "7400") }
        val stored = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(saved, 1), 1).copy(allowClient = false, routeOrder = listOf(Route.CLOUD, Route.CLIENT, Route.SERVER))
        val r = d.rebase(stored)
        assertEquals("7400", r.draft.port)
        assertTrue(r.draft.allowCloud); assertTrue(r.draft.allowCloudCleanup); assertFalse(r.draft.allowClient)
        assertEquals(stored.cloud, r.draft.cloud)
        assertEquals(stored.routeOrder, r.draft.routeOrder)
        assertTrue(r.dirty)
        assertFalse(DevDraft.of(saved, Page.SERVER).rebase(stored).dirty)
        // Saving Server writes the engines and the cloud part back unchanged.
        val out = r.result(caps).settings!!
        assertEquals(stored.cloud, out.cloud); assertEquals(stored.cleanupOrder, out.cleanupOrder); assertTrue(out.allowCloudCleanup)
        assertEquals(stored.routeOrder, out.routeOrder); assertFalse(out.allowClient)
    }

    @Test fun errorsOnOtherPagesAreReportedSeparately() {
        // A stored value out of range (e.g. an old pref) blocks save; the page must say where.
        val bad = DevSettingsForm.Input.of(saved).copy(connectTimeoutMs = "50")
        val d = DevDraft(bad, bad, Page.SERVER).edit { copy(port = "7400") }
        assertFalse(d.canSave(caps))
        assertTrue(DevPages.offPageErrors(d.result(caps), Page.SERVER).isEmpty())
        assertEquals(setOf(Field.CONNECT_TIMEOUT), DevPages.offPageErrors(d.result(caps), Page.ENGINES).keys)
    }

    @Test fun warningsGoToTheirPage() {
        val order = "When your server is unreachable, audio goes to OpenRouter."
        val all = listOf(DevSettingsForm.BOTH_OFF, DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN, DevSettingsForm.CLOUD_NO_CONSENT, order)
        assertEquals(listOf(DevSettingsForm.BOTH_OFF), DevPages.warningsFor(Page.HUB, all))
        assertEquals(listOf(DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN), DevPages.warningsFor(Page.SERVER, all))
        // Engines, consent and the cloud's place: on Settings → Speech to text, in its words.
        assertEquals(listOf(DevPages.NO_ENGINE, DevSettingsForm.CLOUD_NO_CONSENT, order), DevPages.speechWarnings(all))
        // every warning is shown somewhere
        for (w in all) assertTrue(w, Page.entries.any { w in DevPages.warningsFor(it, all) } || w in DevPages.speechWarnings(all))
    }

    // ---- route order ----

    @Test fun moveSwapsVisibleNeighboursOnly() {
        val order = listOf(Route.CLOUD, Route.SERVER, Route.CLIENT)
        val shown = { r: Route -> r != Route.CLOUD }
        assertEquals(listOf(Route.CLOUD, Route.CLIENT, Route.SERVER), DevPages.move(order, Route.CLIENT, -1, shown))
        assertEquals(order, DevPages.move(order, Route.SERVER, -1, shown))
        assertEquals(order, DevPages.move(order, Route.CLIENT, +1, shown))
        assertEquals(order, DevPages.move(order, Route.CLOUD, +1, shown))
    }

    // ---- Speech to text: ranked engines ----

    @Test fun enginesListThisBuildsEnginesInStoredOrderWithANewCloudLast() {
        assertEquals(listOf(Route.SERVER, Route.CLIENT, Route.CLOUD), DevPages.engines(saved, caps))
        assertEquals(listOf(Route.CLIENT, Route.SERVER, Route.CLOUD), DevPages.engines(saved.copy(routeOrder = listOf(Route.CLIENT, Route.SERVER)), caps))
        assertEquals(listOf(Route.CLOUD, Route.SERVER, Route.CLIENT), DevPages.engines(OpenRouterPage.enableStt(saved, 1), caps))
        assertEquals(listOf(Route.CLIENT), DevPages.engines(saved, BuildCaps.LOCAL))
        assertEquals(listOf(Route.SERVER, Route.CLOUD), DevPages.engines(saved, BuildCaps.REMOTE))
    }

    @Test fun engineSwitches() {
        assertTrue(DevPages.engineOn(saved, Route.SERVER)); assertTrue(DevPages.engineOn(saved, Route.CLIENT)); assertFalse(DevPages.engineOn(saved, Route.CLOUD))
        assertFalse(DevPages.withEngine(saved, Route.SERVER, false).allowServer)
        assertFalse(DevPages.withEngine(saved, Route.CLIENT, false).allowClient)
        val on = OpenRouterPage.enableStt(DevPages.placeCloud(saved, caps), 5)
        assertTrue(DevPages.engineOn(on, Route.CLOUD))
        // Turned on from its listed place (3rd), it stays 3rd instead of jumping to the top.
        assertEquals(listOf(Route.SERVER, Route.CLIENT, Route.CLOUD), on.routeOrder)
        val off = DevPages.withEngine(on, Route.CLOUD, false)
        assertFalse(off.allowCloud); assertTrue("consent kept", off.cloud.stt.consentOk)
        assertEquals(on.routeOrder, off.routeOrder)
    }

    @Test fun reorderSwapsListedNeighbours() {
        assertEquals(listOf(Route.CLIENT, Route.SERVER), DevPages.reorder(saved, caps, Route.CLIENT, -1))
        assertEquals(listOf(Route.SERVER, Route.CLIENT), DevPages.reorder(saved, caps, Route.SERVER, -1))
        // Moving the never-placed cloud up puts it in the order; moving another engine leaves it out.
        assertEquals(listOf(Route.SERVER, Route.CLOUD, Route.CLIENT), DevPages.reorder(saved, caps, Route.CLOUD, -1))
        assertEquals(listOf(Route.CLIENT, Route.SERVER), DevPages.reorder(saved, caps, Route.SERVER, +1))
        // An engine this build lacks keeps its stored entry.
        val local = saved.copy(routeOrder = listOf(Route.SERVER, Route.CLIENT, Route.CLOUD))
        assertEquals(listOf(Route.CLIENT, Route.SERVER, Route.CLOUD), DevPages.reorder(local, BuildCaps.LOCAL, Route.CLIENT, -1))
    }

    // ---- hub subtitles ----

    @Test fun routingSummary() {
        // 2026-10-24 phone-local-only: the order must be obvious at a glance ("X first → Y").
        assertEquals("Server first → Phone", DevSummary.routing(saved, caps))
        assertEquals("Phone first → Server", DevSummary.routing(saved.copy(routeOrder = listOf(Route.CLIENT, Route.SERVER)), caps))
        assertEquals("Phone only", DevSummary.routing(saved.copy(allowServer = false), caps))
        assertEquals("Server only", DevSummary.routing(saved.copy(allowClient = false), caps))
        assertEquals("Dictation disabled", DevSummary.routing(saved.copy(allowServer = false, allowClient = false), caps))
        assertEquals("Phone only", DevSummary.routing(DevSettings.defaults(BuildCaps.LOCAL), BuildCaps.LOCAL))
        val c = saved.copy(allowCloud = true, routeOrder = listOf(Route.CLOUD, Route.SERVER, Route.CLIENT))
        assertEquals("Cloud (OpenRouter) first → Server → Phone", DevSummary.routing(c, caps))
    }

    @Test fun routingSummaryFollowsTheStoredOrder() {
        for (order in listOf(listOf(Route.SERVER, Route.CLIENT), listOf(Route.CLIENT, Route.SERVER))) {
            val shown = DevSummary.routing(saved.copy(routeOrder = order), caps).split(" → ").map { it.removeSuffix(" first") }
            assertEquals(order.map { DevSummary.routeName(it, saved) }, shown)
        }
    }

    @Test fun serverSummaryNeverShowsTheToken() {
        val s = DevSummary.server(saved, caps)
        assertEquals("100.64.0.10:7313 · ws:// · token set", s)
        assertFalse(s.contains(saved.token))
        assertEquals("Not set", DevSummary.server(DevSettings.defaults(caps), caps))
        assertEquals("box.ts.net:443 · TLS · pinned · no token", DevSummary.server(saved.copy(host = "box.ts.net", port = 443, tls = true, tlsPinSha256 = "a".repeat(64), token = ""), caps))
        assertEquals("Not in this build", DevSummary.server(saved, BuildCaps.LOCAL))
        assertEquals("Off · 100.64.0.10:7313", DevSummary.server(saved.copy(allowServer = false), caps))
    }

    @Test fun timeoutsSummary() = assertEquals("connect 2500 ms · commit 3000 ms · health 30 s", DevSummary.timeouts(saved.copy(connectTimeoutMs = 2500, commitTimeoutBaseMs = 3000, healthTtlS = 30)))

    @Test fun cloudSummary() {
        assertEquals("Not in this build", DevSummary.cloud(saved, BuildCaps.LOCAL, hasKey = true))
        assertEquals("Speech off · Cleanup off", DevSummary.cloud(saved, caps, hasKey = true))
        val noConsent = saved.copy(allowCloud = true, routeOrder = DevSettings.withCloudFirst(saved.routeOrder))
        assertEquals("Speech on, no consent · Cleanup off · no key", DevSummary.cloud(noConsent, caps, hasKey = false))
        val ok = OpenRouterPage.enableStt(saved, 1).let { it.copy(cloud = it.cloud.copy(monthlyBudgetUsd = 5.0)) }
        assertEquals("Speech on · Cleanup off · budget \$5.00", DevSummary.cloud(ok, caps, hasKey = true))
    }

    @Test fun debugSummary() {
        assertEquals("Log off · audio off", DevSummary.debug(log = false, audio = false))
        assertEquals("Log on · audio kept", DevSummary.debug(log = true, audio = true))
    }

    @Test fun statusCardLastDictation() {
        assertNull(DevSummary.lastShort(null))
        val r = CommitResult(CommitStatus.FORMATTED, timings = Timings(totalMs = 900), route = RouteInfo(engine = "remote", server = "100.64.0.10:7313", clientMs = 412))
        assertEquals("Server · client 412 ms", DevSummary.lastShort(r))
        val l = CommitResult(CommitStatus.FORMATTED, timings = Timings(totalMs = 650), route = RouteInfo(engine = "local", fallback = "connect_failed"))
        assertEquals("On-device (fallback: connect_failed) · total 650 ms", DevSummary.lastShort(l))
    }

    @Test fun engineOptionsSummary() {
        assertEquals("Badge off", DevSummary.engineOptions(false, null))
        assertEquals("Badge on · Parakeet greedy", DevSummary.engineOptions(true, "greedy"))
    }
}
