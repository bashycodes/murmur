package dev.murmur.core

import dev.murmur.admin.AdminCode
import dev.murmur.admin.AdminJson
import dev.murmur.admin.AdminResult
import dev.murmur.admin.UsersKeys
import dev.murmur.bubble.RouteLabel
import dev.murmur.data.BuildCaps
import dev.murmur.data.CleanupEngines
import dev.murmur.data.DevSettings
import dev.murmur.data.ServerHealth
import dev.murmur.data.SettingsSummary
import dev.murmur.session.DictationController
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * D112: a server lockout (`throttled`, HTTP 429 after too many wrong keys) reads as its own
 * sentence with the wait on every surface, never as "Server unreachable" / "Not reachable".
 */
class LockoutTest {
    private val in42 = "Too many wrong keys from this address. Try again in 42 s. Your key isn't checked until then."
    private val later = "Too many wrong keys from this address. Try again later. Your key isn't checked until then."
    private val saved = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = "tok_test")

    @Test fun theSentenceMatchesTheRouters() {
        assertEquals(in42, Lockout.text(42))
        assertEquals(later, Lockout.text(null))
        assertEquals("Too many wrong keys from this address. The wait is over: try again.", Lockout.text(0))
    }

    @Test fun theWaitCountsDownFromTheProbe() {
        val h = HealthReport(ok = false, error = Lockout.CODE, retryAfterS = 60, checkedAtMs = 1_000_000)
        assertEquals(60L, Lockout.left(h, 1_000_000))
        assertEquals(15L, Lockout.left(h, 1_045_500))
        assertEquals(0L, Lockout.left(h, 1_090_000))
        assertEquals(Lockout.text(15), Lockout.text(h, 1_045_500))
        // A clock behind the probe, or a huge wait: never more than the cap.
        assertEquals(60L, Lockout.left(h, 500))
        assertEquals(Lockout.MAX_WAIT_S, Lockout.left(h.copy(retryAfterS = Long.MAX_VALUE, checkedAtMs = null), 0))
        assertNull(Lockout.left(h.copy(retryAfterS = null), 0))
    }

    @Test fun aThrottledProbeDecodesWithItsWait() {
        val h = RouterJson.decodeHealth(
            """{"ok":false,"engine":"remote","error":"throttled","message":"m","retry_after_s":42}""",
        )
        assertEquals(Lockout.CODE, h.error)
        assertEquals(42L, h.retryAfterS)
        assertEquals(in42, DevText.probe(h))
        assertNull(RouterJson.decodeHealth("""{"ok":false,"error":"connect_failed"}""").retryAfterS)
        // The Settings rows say it too, instead of "Not reachable".
        assertEquals(in42, SettingsSummary.server(saved, BuildCaps.ALL, h))
        assertEquals(in42, CleanupEngines.serverLine(saved, BuildCaps.ALL, null, circuitOpen = true, h = h))
    }

    @Test fun aLockoutStaysFreshForItsWholeWait() {
        val cache = ServerHealth()
        cache.record(saved, HealthReport(ok = false, error = Lockout.CODE, retryAfterS = 600), 1_000)
        assertNotNull("no re-probe inside the lockout", cache.fresh(saved, 1_000 + 500_000))
        assertNull(cache.fresh(saved, 1_000 + 600_001))
        cache.record(saved, HealthReport(ok = false, error = Lockout.CODE), 1_000)
        assertNull(cache.fresh(saved, 1_000 + ServerHealth.MAX_AGE_MS + 1))
        // A huge Retry-After neither overflows nor parks the check past the cap.
        val huge = HealthReport(ok = false, error = Lockout.CODE, retryAfterS = Long.MAX_VALUE)
        assertEquals(Lockout.MAX_WAIT_S * 1000, ServerHealth.maxAge(huge))
    }

    @Test fun settingsStaysQuietDuringALockout() {
        // SettingsActivity.checkServer probes only when this says so.
        val locked = ServerHealth.Probe(HealthReport(ok = false, error = Lockout.CODE, retryAfterS = 300), 1_000)
        assertFalse(ServerHealth.needsProbe(locked, 1_000 + 120_000))
        assertTrue(ServerHealth.needsProbe(locked, 1_000 + 300_001))
        val down = ServerHealth.Probe(HealthReport(ok = false, error = "connect_failed"), 1_000)
        assertTrue(ServerHealth.needsProbe(down, 1_000 + ServerHealth.MAX_AGE_MS + 1))
        assertTrue(ServerHealth.needsProbe(null, 1_000))
    }

    @Test fun theRoutePillNamesTheLockout() {
        val r = SessionRoute(engine = "local", fallback = Lockout.CODE)
        assertEquals("via phone · too many wrong keys", RouteLabel.of(r))
    }

    @Test fun aStartErrorIsTheSentenceNotServerUnreachable() {
        assertEquals(in42, RouterErrors.userMessage(RouterErrors.THROTTLED, in42))
        assertEquals(later, RouterErrors.userMessage(RouterErrors.THROTTLED, ""))
        assertEquals(RouterErrors.THROTTLED, CoreException.of(in42, "throttled", "x").kind)
        // The kind comes from the route's fallback code, not the sentence.
        val failed = CommitResult(CommitStatus.ERROR, error = in42, route = RouteInfo(engine = "remote", fallback = Lockout.CODE))
        assertEquals(RouterErrors.THROTTLED, DictationController.commitKind(failed))
        assertEquals(in42, DictationController.routerFailure(failed))
        assertNull(DictationController.routerFailure(failed.copy(route = null)))
        assertEquals(in42, DictationController.withClientNote(in42, RouterErrors.THROTTLED, "no phone model"))
    }

    @Test fun usersAndKeysShowsTheWait() {
        val r = AdminJson.decode("""{"ok":false,"code":"throttled","message":"too many","retry_after_s":42}""") { it }
        assertEquals(AdminResult.Err(AdminCode.THROTTLED, "too many", 42), r)
        assertEquals(in42, UsersKeys.errorText(r as AdminResult.Err))
        assertEquals(later, UsersKeys.errorText(AdminCode.THROTTLED, ""))
        val plain = AdminJson.decode("""{"ok":false,"code":"forbidden","message":"no"}""") { it } as AdminResult.Err
        assertNull(plain.retryAfterS)
    }
}
