package dev.murmur.policy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Our own screens stay bubble-free (settings fields) except a dictation surface like the Scratchpad. */
class OwnAppGateTest {
    private val own = "dev.murmur"

    @Test fun ownPackageIsBlockedUnlessADictationSurfaceIsInFront() {
        assertTrue(OwnAppGate.blocks(own, own, surfaceInFront = false))
        assertFalse(OwnAppGate.blocks(own, own, surfaceInFront = true))
    }

    @Test fun otherAppsAreNeverBlockedByThisGate() {
        assertFalse(OwnAppGate.blocks("com.example.notes", own, surfaceInFront = false))
        assertFalse(OwnAppGate.blocks("com.example.notes", own, surfaceInFront = true))
        assertFalse(OwnAppGate.blocks(null, own, surfaceInFront = false))
    }
}
