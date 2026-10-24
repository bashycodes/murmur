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
        assertEquals(Page.CLOUD, DevPages.pageOf(Field.CLOUD_BUDGET))
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
        val e = DevDraft.of(saved, Page.ROUTING).edit { copy(keepLocalWarm = false, routeOrder = listOf(Route.CLIENT, Route.SERVER)) }
        val want = DevSettingsForm.validate(DevSettingsForm.Input.of(saved).copy(keepLocalWarm = false, routeOrder = listOf(Route.CLIENT, Route.SERVER)), caps).settings
        assertEquals(want, e.result(caps).settings)
        assertEquals(saved.token, e.result(caps).settings!!.token)
    }

    @Test fun rebaseKeepsThisPagesEditsAndTakesTheRestFromTheStore() {
        val d = DevDraft.of(saved, Page.CLOUD).edit { copy(cloudBudget = "5") }
        val p = CloudProvider(id = "p1", baseUrl = "https://api.example.com/v1", label = "Ex")
        val stored = saved.copy(cloud = CloudSettings(active = "p1", providers = listOf(p)), host = "10.0.0.9")
        val r = d.rebase(stored)
        assertEquals("5", r.draft.cloudBudget)
        assertEquals(listOf(p), r.draft.cloud.providers)
        assertEquals("10.0.0.9", r.draft.host)
        assertTrue(r.dirty)
        assertFalse(DevDraft.of(saved, Page.CLOUD).rebase(stored).dirty)
    }

    @Test fun errorsOnOtherPagesAreReportedSeparately() {
        // A stored value out of range (e.g. an old pref) blocks save; the page must say where.
        val bad = DevSettingsForm.Input.of(saved).copy(connectTimeoutMs = "50")
        val d = DevDraft(bad, bad, Page.ROUTING).edit { copy(keepLocalWarm = false) }
        assertFalse(d.canSave(caps))
        val off = DevPages.offPageErrors(d.result(caps), Page.ROUTING)
        assertEquals(setOf(Field.CONNECT_TIMEOUT), off.keys)
        assertTrue(DevPages.offPageErrors(d.result(caps), Page.SERVER).isEmpty())
    }

    @Test fun warningsGoToTheirPage() {
        val order = "When your server is unreachable, audio goes to Groq."
        val all = listOf(DevSettingsForm.BOTH_OFF, DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN, DevSettingsForm.CLOUD_NO_CONSENT, order)
        assertEquals(listOf(DevSettingsForm.BOTH_OFF), DevPages.warningsFor(Page.HUB, all))
        assertEquals(listOf(DevSettingsForm.BOTH_OFF, order), DevPages.warningsFor(Page.ROUTING, all))
        assertEquals(listOf(DevSettingsForm.NO_HOST, DevSettingsForm.PLAINTEXT, DevSettingsForm.NO_TOKEN), DevPages.warningsFor(Page.SERVER, all))
        assertEquals(listOf(DevSettingsForm.CLOUD_NO_CONSENT, order), DevPages.warningsFor(Page.CLOUD, all))
        // every warning is shown somewhere
        for (w in all) assertTrue(w, Page.entries.any { w in DevPages.warningsFor(it, all) })
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

    // ---- hub subtitles ----

    @Test fun routingSummary() {
        // 2026-10-24 phone-local-only: the order must be obvious at a glance ("X first → Y").
        assertEquals("Server first → Phone", DevSummary.routing(saved, caps))
        assertEquals("Phone first → Server", DevSummary.routing(saved.copy(routeOrder = listOf(Route.CLIENT, Route.SERVER)), caps))
        assertEquals("Phone only", DevSummary.routing(saved.copy(allowServer = false), caps))
        assertEquals("Server only", DevSummary.routing(saved.copy(allowClient = false), caps))
        assertEquals("Dictation disabled", DevSummary.routing(saved.copy(allowServer = false, allowClient = false), caps))
        assertEquals("Phone only", DevSummary.routing(DevSettings.defaults(BuildCaps.LOCAL), BuildCaps.LOCAL))
        val p = CloudProvider(id = "p1", label = "Groq", baseUrl = "https://api.groq.com/openai/v1")
        val c = saved.copy(allowCloud = true, routeOrder = listOf(Route.CLOUD, Route.SERVER, Route.CLIENT), cloud = CloudSettings("p1", listOf(p)))
        assertEquals("Cloud (Groq) first → Server → Phone", DevSummary.routing(c, caps))
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
        assertEquals("Not in this build", DevSummary.cloud(saved, BuildCaps.LOCAL))
        assertEquals("Off · no providers", DevSummary.cloud(saved, caps))
        val p = CloudProvider(id = "p1", label = "Groq", baseUrl = "https://api.groq.com/openai/v1")
        val two = saved.copy(cloud = CloudSettings("p1", listOf(p, p.copy(id = "p2"))))
        assertEquals("Off · 2 providers", DevSummary.cloud(two, caps))
        assertEquals("On · Groq · no consent", DevSummary.cloud(two.copy(allowCloud = true), caps))
        val ok = two.copy(allowCloud = true, cloud = two.cloud.replace(p.withConsent(1)).copy(monthlyBudgetUsd = 5.0))
        assertEquals("On · Groq · budget \$5.00", DevSummary.cloud(ok, caps))
        assertEquals("On · no active provider", DevSummary.cloud(saved.copy(allowCloud = true), caps))
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
}
