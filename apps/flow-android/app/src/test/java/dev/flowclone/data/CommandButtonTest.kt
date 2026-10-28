package dev.flowclone.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CommandButtonTest {
    @Test fun withSelectionIsTheDefaultAndShowsOnlyWithASelection() {
        assertEquals(CommandButton.WITH_SELECTION, Settings().commandButton)
        assertTrue(CommandButton.WITH_SELECTION.shows(hasSelection = true))
        assertFalse(CommandButton.WITH_SELECTION.shows(hasSelection = false))
    }

    @Test fun alwaysAndOff() {
        assertTrue(CommandButton.ALWAYS.shows(false))
        assertFalse(CommandButton.OFF.shows(true))
    }

    @Test fun storedKeysRoundTripAndUnknownIsTheDefault() {
        for (b in CommandButton.entries) assertEquals(b, CommandButton.fromKey(b.key))
        assertEquals(CommandButton.WITH_SELECTION, CommandButton.fromKey(null))
        assertEquals(CommandButton.WITH_SELECTION, CommandButton.fromKey("bogus"))
    }
}
