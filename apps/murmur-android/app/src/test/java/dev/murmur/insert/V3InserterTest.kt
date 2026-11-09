package dev.flowclone.insert

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class V3InserterTest {
    private val env = FakeEnv()
    private val field get() = env.field

    private suspend fun TestScope.insert(text: String, t0: Long = currentTime): InsertResult =
        V3Inserter(env) { currentTime }.insert(text, t0)

    @Test fun emptyTextSucceedsWithoutCommit() = runTest {
        val r = insert("")
        assertTrue(r.succeeded)
        assertTrue(field.commits.isEmpty())
    }

    @Test fun happyPathCommitsOnceAndVerifiesViaSurroundingText() = runTest {
        val r = insert("hello")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_TEXT_FOUND, r.status)
        assertEquals(InsertResult.VerifySource.SURROUNDING_TEXT, r.source)
        assertEquals(listOf("hello"), field.commits)
        assertEquals("hello", field.text)
        assertTrue("verified within one settle step", currentTime <= 100)
    }

    @Test fun sensitiveEditorTypeIsRefused() = runTest {
        env.editorType = 0x81 // TEXT_PASSWORD
        val r = insert("secret")
        assertFalse(r.succeeded)
        assertEquals(InsertResult.FailureReason.EXCLUDED_FIELD, r.reason)
        assertTrue(field.commits.isEmpty())
        assertTrue(field.setTexts.isEmpty())
    }

    @Test fun replyContextNeverCommitsAndUsesSetText() = runTest {
        env.replyContext = true
        val r = insert("on my way")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_VIA_ACTION_SET_TEXT, r.status)
        assertTrue(field.commits.isEmpty())
        assertEquals(listOf("on my way"), field.setTexts)
        assertTrue("no focus nudges in reply context", env.focusActions.isEmpty())
    }

    @Test fun noConnectionWaitsThenFallsBackToNodeTierOnEmptyField() = runTest {
        env.sessionActive = false; env.connection = null
        val r = insert("hi")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_VIA_ACTION_SET_TEXT, r.status)
        assertEquals("session wait is 500 ms", 500L + 100, currentTime)
        assertTrue("plain app is not a reacquire surface", env.focusActions.isEmpty())
    }

    @Test fun noConnectionAndNonEmptyFieldFails() = runTest {
        env.sessionActive = false; env.connection = null; field.text = "existing"
        val r = insert("hi")
        assertFalse(r.succeeded)
        assertEquals("existing", field.text)
    }

    @Test fun noConnectionOnLauncherNudgesFocusAndCommitsAfterRestart() = runTest {
        val f = FakeField(pkg = "com.google.android.apps.nexuslauncher")
        env.bindTo(f); env.currentPkg = f.pkg; env.startPkg = f.pkg
        env.sessionActive = false; env.connection = null
        env.onNudge = { bindTo(f) }
        val r = insert("weather")
        assertTrue(r.succeeded)
        assertEquals(listOf("FOCUS"), env.focusActions)
        assertEquals(listOf("weather"), f.commits)
    }

    @Test fun sessionMismatchOnOurPackageReacquiresThenCommits() = runTest {
        env.boundPkg = "com.other.app"
        env.onNudge = { boundPkg = field.pkg }
        val r = insert("hi")
        assertTrue(r.succeeded)
        assertEquals(listOf("FOCUS"), env.focusActions)
        assertEquals(listOf("hi"), field.commits)
    }

    @Test fun sessionMismatchOnForeignPackageGoesToNodeTierWithoutCommit() = runTest {
        env.boundPkg = "com.other.app"; env.focusedPkg = "com.third.app"
        val r = insert("hi")
        assertTrue(field.commits.isEmpty())
        assertTrue(env.focusActions.isEmpty())
        // node package (com.example.notes) is still ours, field empty -> SET_TEXT
        assertEquals(InsertResult.Status.SUCCESS_VIA_ACTION_SET_TEXT, r.status)
    }

    @Test fun stalePrecommitProbeInChatAppEscalatesToClearFocus() = runTest {
        val dead = FakeField(pkg = "com.whatsapp").apply { surroundingNull = true }
        val fresh = FakeField(pkg = "com.whatsapp")
        env.bindTo(dead); env.currentPkg = "com.whatsapp"; env.startPkg = "com.whatsapp"
        env.onNudge = { bindTo(fresh) }
        val r = insert("hey")
        assertTrue(r.succeeded)
        assertEquals(listOf("CLEAR_FOCUS+FOCUS"), env.focusActions)
        assertTrue(dead.commits.isEmpty())
        assertEquals(listOf("hey"), fresh.commits)
    }

    @Test fun stalePrecommitProbeNotRunOutsideRestartProneApps() = runTest {
        field.surroundingNull = true
        insert("x")
        assertTrue(env.focusActions.isEmpty())
        assertEquals(listOf("x"), field.commits)
    }

    @Test fun trustedNullReadInKnownNullApp() = runTest {
        // Gmail: known-null but not restart-prone, so no stale probe precedes the commit.
        val f = FakeField(pkg = "com.google.android.gm").apply { surroundingNull = true; nodeTextNull = true }
        env.bindTo(f); env.currentPkg = f.pkg; env.startPkg = f.pkg
        val r = insert("thanks")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_NULL_SURROUNDING_TRUSTED_COMMIT, r.status)
        assertEquals(InsertResult.VerifySource.TRUSTED, r.source)
    }

    @Test fun allNullReadsInUnknownAppFallToNodeTierAndFail() = runTest {
        field.surroundingNull = true; field.nodeTextNull = true
        val r = insert("x")
        assertFalse(r.succeeded)
        assertEquals(listOf("x"), field.commits)
        assertTrue(field.setTexts.isEmpty())
        assertTrue("verify window is 700 ms", currentTime in 700..800)
    }

    @Test fun silentCommitFailureFallsBackToSetTextOnEmptyField() = runTest {
        field.commitSilentlyDropped = true
        val r = insert("hello")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_VIA_ACTION_SET_TEXT, r.status)
        assertEquals("commitText exactly once", 1, field.commits.size)
        assertEquals("hello", field.text)
    }

    @Test fun silentCommitIntoNonEmptyFieldFailsWithoutOverwrite() = runTest {
        field.text = "draft "; field.commitSilentlyDropped = true
        val r = insert("hello")
        assertFalse(r.succeeded)
        assertEquals("draft ", field.text)
        assertTrue(field.setTexts.isEmpty())
    }

    @Test fun lateLandingCommitIsAlreadyPresentNotDoubleInserted() = runTest {
        // commit "lands" but surrounding text lies (stale null); node shows the text.
        field.surroundingNull = true
        val r = insert("hello")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_TEXT_FOUND_VIA_FALLBACK, r.status)
        assertTrue(field.setTexts.isEmpty())
        assertEquals("hello", field.text)
    }

    @Test fun placeholderTextCountsAsEmptyForSetText() = runTest {
        env.replyContext = true
        field.text = "Reply"; field.hint = "reply"
        val r = insert("ok")
        assertTrue(r.succeeded)
        assertEquals("ok", field.text)
    }

    @Test fun whatsappComposePlaceholderTreatedEmptyAfterStaleProbe() = runTest {
        val f = FakeField(text = "Message", pkg = "com.whatsapp").apply { commitSilentlyDropped = true }
        env.bindTo(f); env.currentPkg = f.pkg; env.startPkg = f.pkg
        val r = insert("hi")
        assertTrue(r.succeeded)
        assertEquals(listOf("hi"), f.setTexts)
    }

    @Test fun nodeWithoutSetTextActionFails() = runTest {
        env.replyContext = true; field.canSetText = false
        assertFalse(insert("x").succeeded)
    }

    @Test fun passwordNodeIsRefusedInNodeTier() = runTest {
        env.replyContext = true; field.isPassword = true
        val r = insert("x")
        assertFalse(r.succeeded)
        assertTrue(field.setTexts.isEmpty())
    }

    @Test fun nodeFromForeignPackageIsRefused() = runTest {
        env.replyContext = true
        env.focusedNode = FakeField(pkg = "com.evil")
        env.startPkg = "com.example.notes"; env.currentPkg = "com.example.notes"
        assertFalse(insert("x").succeeded)
    }

    /** S12: the node tier no longer accepts whatever app is focused now, only the pinned start app. */
    @Test fun nodeOfTheNowFocusedAppIsRefusedWhenTheStartAppDiffers() = runTest {
        env.replyContext = true
        val other = FakeField(pkg = "com.whatsapp")
        env.focusedNode = other
        env.startPkg = "com.example.notes"; env.currentPkg = "com.whatsapp"
        assertFalse(insert("x").succeeded)
        assertTrue(other.setTexts.isEmpty())
    }

    @Test fun bankingAppFocusedAtInsertTimeIsRefused() = runTest {
        env.currentPkg = "com.chase.sig.android"
        val r = insert("x")
        assertFalse(r.succeeded)
        assertEquals(InsertResult.FailureReason.EXCLUDED_FIELD, r.reason)
        assertTrue(field.commits.isEmpty() && field.setTexts.isEmpty())
    }

    @Test fun relocatedDictationStartNodeIsUsedWhenFocusLost() = runTest {
        env.replyContext = true; env.focusedNode = null; env.relocatedNode = field
        assertTrue(insert("x").succeeded)
        assertEquals("x", field.text)
    }

    @Test fun commitThatThrowsFallsToNodeTier() = runTest {
        field.commitThrows = true
        val r = insert("x")
        assertTrue(r.succeeded)
        assertEquals(InsertResult.Status.SUCCESS_VIA_ACTION_SET_TEXT, r.status)
    }

    @Test fun excludedTypeRecheckedAfterRefocus() = runTest {
        env.boundPkg = "com.other.app"
        env.onNudge = { boundPkg = field.pkg; editorType = 0x12 /* NUMBER_PASSWORD */ }
        val r = insert("x")
        assertFalse(r.succeeded)
        assertEquals(InsertResult.FailureReason.EXCLUDED_FIELD, r.reason)
        assertTrue(field.commits.isEmpty())
    }

    @Test fun mutexWaitEatsBudgetAndBlocksCommitWhenUnder300msLeft() = runTest {
        // t0 was 3300 ms ago (queued behind another insertion).
        kotlinx.coroutines.delay(3300)
        val r = insert("x", t0 = 0)
        assertFalse(r.succeeded)
        assertEquals(InsertResult.Status.INCONCLUSIVE_FAILURE, r.status)
        assertTrue(field.commits.isEmpty())
    }

    @Test fun deadlineAlreadyPassedFailsImmediately() = runTest {
        kotlinx.coroutines.delay(4000)
        assertFalse(insert("x", t0 = 0).succeeded)
        assertTrue(field.commits.isEmpty())
    }

    @Test fun worstCaseStaysWithinDeadline() = runTest {
        val dead = FakeField(pkg = "com.whatsapp").apply { surroundingNull = true; nodeTextNull = true }
        env.bindTo(dead); env.currentPkg = "com.whatsapp"; env.startPkg = "com.whatsapp"
        env.sessionActive = false; env.connection = null
        env.onNudge = { sessionActive = true; connection = dead }
        env.nudgeStartDelayMs = 450
        insert("x")
        assertTrue("took ${currentTime} ms", currentTime <= 3500 + 200)
    }

    @Test fun verifyDetectsTextAppearingLateInWindow() = runTest {
        // IME applies the commit asynchronously: text shows up on the 4th read.
        var reads = 0
        val lagging = object : InputConnectionPort by field {
            override fun getSurroundingText(before: Int, after: Int): SurroundingText? {
                reads++
                return SurroundingText(if (reads >= 4) field.text else "")
            }
        }
        env.connection = lagging
        field.contentDesc = null
        // Node also lags so the fallback read doesn't short-circuit.
        val r = insert("late")
        assertTrue(r.succeeded)
        assertEquals(4, reads)
        assertEquals(InsertResult.Status.SUCCESS_TEXT_FOUND, r.status)
    }
}
