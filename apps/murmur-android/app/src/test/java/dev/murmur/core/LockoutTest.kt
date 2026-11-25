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
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * D112: a server lockout (`throttled`, HTTP 429 after too many wrong keys) reads as its own
 * sentence with the wait on every surface, never as "Server unreachable" / "Not reachable".
 */
class LockoutTest {
    private val in42 = "Too many wrong keys from this address. Try again in 42 s."
    private val saved = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = "tok_test")

    @Test fun theSentenceMatchesTheRouters() {
        assertEquals(in42, Lockout.text(42))
        assertEquals("Too many wrong keys from this address. Try again later.", Lockout.text(null))
    }

    @Test fun aThrottledProbeDecodesWithItsWait() {
        val h = RouterJson.decodeHealth(
            """{"ok":false,"engine":"remote","error":"throttled","message":"$in42","retry_after_s":42,"checked_at_ms":5}""",
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
    }

    @Test fun theRoutePillNamesTheLockout() {
        val r = SessionRoute(engine = "local", fallback = Lockout.CODE)
        assertEquals("via phone · too many wrong keys", RouteLabel.of(r))
    }

    @Test fun aStartErrorIsTheSentenceNotServerUnreachable() {
        assertEquals(in42, RouterErrors.userMessage(RouterErrors.THROTTLED, in42))
        assertEquals(Lockout.text(null), RouterErrors.userMessage(RouterErrors.THROTTLED, ""))
        assertEquals(RouterErrors.THROTTLED, CoreException.of(in42, "throttled", "x").kind)
        // A library without nativeLastErrorKind: recovered from the sentence.
        assertEquals(RouterErrors.THROTTLED, CoreException.kindOf(in42))
        // A commit after the failed start carries the same sentence.
        val failed = CommitResult(CommitStatus.ERROR, error = in42)
        assertEquals(in42, DictationController.routerFailure(failed))
        assertEquals(in42, DictationController.withClientNote(in42, RouterErrors.THROTTLED, "no phone model"))
    }

    @Test fun usersAndKeysShowsTheWait() {
        val r = AdminJson.decode("""{"ok":false,"code":"throttled","message":"too many","retry_after_s":42}""") { it }
        assertEquals(AdminResult.Err(AdminCode.THROTTLED, "too many", 42), r)
        assertEquals(in42, UsersKeys.errorText(r as AdminResult.Err))
        assertEquals(Lockout.text(null), UsersKeys.errorText(AdminCode.THROTTLED, ""))
        val plain = AdminJson.decode("""{"ok":false,"code":"forbidden","message":"no"}""") { it } as AdminResult.Err
        assertNull(plain.retryAfterS)
    }
}
