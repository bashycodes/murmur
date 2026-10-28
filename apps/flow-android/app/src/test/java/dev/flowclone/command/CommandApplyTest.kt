package dev.flowclone.command

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CommandApplyTest {
    private class Field(var text: String?, override val canSetText: Boolean = true) : CommandField {
        var setTextOk = true
        var cursor: Int? = null
        var imeSelection: String? = null
        val commits = mutableListOf<String>()
        /** An editor that rewrites what it is given (e.g. drops it). */
        var mangle: ((String) -> String)? = null
        override fun readText() = text
        override fun setText(text: String): Boolean {
            if (!setTextOk) return false
            this.text = mangle?.invoke(text) ?: text
            return true
        }
        override fun setCursor(at: Int): Boolean { cursor = at; return true }
        override fun imeSelectedText() = imeSelection
        override fun imeCommit(text: String): Boolean { commits += text; return true }
    }

    private val original = "Hi team. The meeting we planned for tomorrow moves to Friday. Thanks!"
    private val sel = "The meeting we planned for tomorrow moves to Friday."
    private val snap = SelectionSnapshot.of(original, original.indexOf(sel), original.indexOf(sel) + sel.length)!!

    @Test fun makeItShorterReplacesOnlyTheSelection() {
        val f = Field(original)
        assertTrue(CommandApply.replaceSelection(snap, "Meeting moves to Friday.", f))
        assertEquals("Hi team. Meeting moves to Friday. Thanks!", f.text)
        assertEquals("Hi team. Meeting moves to Friday.".length, f.cursor)
        assertTrue(f.commits.isEmpty())
    }

    @Test fun aMovedSelectionTouchesNothing() {
        val f = Field("Completely different text now").apply { imeSelection = sel }
        assertFalse(CommandApply.replaceSelection(snap, "X", f))
        assertEquals("Completely different text now", f.text)
        assertTrue("no IME guess either", f.commits.isEmpty())
    }

    @Test fun withoutSetTextTheImeReplacesTheStillSelectedText() {
        val f = Field(original, canSetText = false).apply { imeSelection = sel }
        assertTrue(CommandApply.replaceSelection(snap, "Short.", f))
        assertEquals(listOf("Short."), f.commits)
        assertEquals("node untouched", original, f.text)
    }

    @Test fun aRefusedSetTextFallsBackToTheIme() {
        val f = Field(original).apply { setTextOk = false; imeSelection = sel }
        assertTrue(CommandApply.replaceSelection(snap, "Short.", f))
        assertEquals(listOf("Short."), f.commits)
    }

    @Test fun theImeOnlyWritesWhileTheSameTextIsSelected() {
        val other = Field(original, canSetText = false).apply { imeSelection = "Thanks!" }
        assertFalse(CommandApply.replaceSelection(snap, "Short.", other))
        val none = Field(original, canSetText = false)
        assertFalse(CommandApply.replaceSelection(snap, "Short.", none))
        assertTrue(other.commits.isEmpty() && none.commits.isEmpty())
    }

    @Test fun aWriteTheFieldDidNotKeepIsAFailure() {
        val f = Field(original).apply { mangle = { "" } }
        assertFalse(CommandApply.replaceSelection(snap, "Short.", f))
    }

    // ---- CommandGuard: what Command Mode may read ----

    private val textType = 0x1 // TYPE_CLASS_TEXT
    private fun refusal(type: Int = textType, password: Boolean = false, sensitive: Boolean = false, pkg: String? = "com.example.notes", deny: Set<String> = emptySet(), field: Boolean = true) =
        CommandGuard.refusal(field, type, password, sensitive, pkg, deny)

    @Test fun anOrdinaryTextFieldIsAllowed() = assertNull(refusal())

    @Test fun passwordAndSensitiveFieldsAreRefused() {
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x81)) // textPassword
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x91)) // textVisiblePassword
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x12)) // numberPassword
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(password = true))
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(sensitive = true))
        assertEquals("numbers / OTP", CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x2))
        assertEquals("phone", CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x3))
        assertEquals("URI", CommandTarget.PROTECTED_MESSAGE, refusal(type = 0x11))
    }

    @Test fun bankingAndDenylistedAppsAreRefused() {
        val bank = dev.flowclone.policy.PolicyLists.RESTRICTED_BANKING.first()
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(pkg = bank))
        assertEquals(CommandTarget.PROTECTED_MESSAGE, refusal(deny = setOf("com.example.notes")))
    }

    @Test fun noFieldIsRefused() = assertEquals(CommandTarget.NO_FIELD_MESSAGE, refusal(field = false))
}
