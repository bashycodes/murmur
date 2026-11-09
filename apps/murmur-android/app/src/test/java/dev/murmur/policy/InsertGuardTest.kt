package dev.flowclone.policy

import dev.flowclone.session.InsertTarget
import org.junit.Assert.assertEquals
import org.junit.Test

/** Review S12: insert only into the field pinned at dictation start, and only while it is allowed. */
class InsertGuardTest {
    private val notes = InsertGuard.Field("com.example.notes", 11)

    @Test fun sameFieldIsOk() = assertEquals(InsertTarget.OK, InsertGuard.check(notes, notes, policyAllows = true, explicit = false))

    @Test fun otherAppOrFieldIsChanged() {
        assertEquals(InsertTarget.CHANGED, InsertGuard.check(notes, InsertGuard.Field("com.whatsapp", 11), true, false))
        assertEquals(InsertTarget.CHANGED, InsertGuard.check(notes, InsertGuard.Field("com.example.notes", 12), true, false))
    }

    @Test fun protectedBeatsEverythingEvenAnExplicitPaste() {
        assertEquals(InsertTarget.PROTECTED, InsertGuard.check(notes, notes, policyAllows = false, explicit = false))
        assertEquals(InsertTarget.PROTECTED, InsertGuard.check(notes, InsertGuard.Field("com.bank", 1), false, explicit = true))
    }

    @Test fun explicitPasteGoesToTheFieldFocusedNow() =
        assertEquals(InsertTarget.OK, InsertGuard.check(notes, InsertGuard.Field("com.whatsapp", 5), true, explicit = true))

    @Test fun unknownIdentityFallsBackToThePackage() {
        assertEquals(InsertTarget.OK, InsertGuard.check(notes, InsertGuard.Field("com.example.notes", null), true, false))
        assertEquals(InsertTarget.OK, InsertGuard.check(null, InsertGuard.Field("com.x", 1), true, false))
    }
}
