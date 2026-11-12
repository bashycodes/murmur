package dev.murmur.policy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Our own screens stay bubble-free (settings fields) except a dictation screen like the Scratchpad. */
class OwnAppGateTest {
    private val own = "dev.murmur"

    @Test fun ownPackageIsBlockedUnlessADictationScreenIsInFront() {
        assertTrue(OwnAppGate.blocks(own, own, dictationScreenInFront = false))
        assertFalse(OwnAppGate.blocks(own, own, dictationScreenInFront = true))
    }

    @Test fun otherAppsAreNeverBlockedByThisGate() {
        assertFalse(OwnAppGate.blocks("com.example.notes", own, dictationScreenInFront = false))
        assertFalse(OwnAppGate.blocks("com.example.notes", own, dictationScreenInFront = true))
        assertFalse(OwnAppGate.blocks(null, own, dictationScreenInFront = false))
    }
}
