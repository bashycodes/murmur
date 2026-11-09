package dev.flowclone.command

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SelectionSpliceTest {
    private val text = "Hi team. The meeting that we planned for tomorrow is moved to Friday. Thanks!"
    private val sel = "The meeting that we planned for tomorrow is moved to Friday."
    private val start = text.indexOf(sel)
    private val snap = SelectionSnapshot.of(text, start, start + sel.length)!!

    @Test fun snapshotReadsTheSelectedRange() {
        assertEquals(sel, snap.selected)
        assertTrue(snap.hasSelection)
        assertEquals("Hi team. ", snap.before)
        assertEquals(" Thanks!", snap.after)
    }

    @Test fun aBackwardsSelectionIsNormalised() {
        val s = SelectionSnapshot.of(text, start + sel.length, start)!!
        assertEquals(snap, s)
    }

    @Test fun badIndicesAreClampedOrRefused() {
        assertNull("no text", SelectionSnapshot.of(null, 0, 1))
        assertNull("no selection info", SelectionSnapshot.of("abc", -1, -1))
        val clamped = SelectionSnapshot.of("abc", 1, 99)!!
        assertEquals("bc", clamped.selected)
        val caret = SelectionSnapshot.of("abc", 2, 2)!!
        assertFalse(caret.hasSelection)
        assertEquals("", caret.selected)
    }

    @Test fun replacesOnlyTheSelectedRangeAndPutsTheCursorAfterIt() {
        val p = SelectionSplice.plan(snap, text, "Meeting moved to Friday.") as SelectionSplice.Plan.SetText
        assertEquals("Hi team. Meeting moved to Friday. Thanks!", p.text)
        assertEquals("Hi team. Meeting moved to Friday.".length, p.cursor)
    }

    @Test fun aSelectionAtTheEdgesKeepsTheRest() {
        val whole = SelectionSnapshot.of("abc", 0, 3)!!
        assertEquals("XY", (SelectionSplice.plan(whole, "abc", "XY") as SelectionSplice.Plan.SetText).text)
        val head = SelectionSnapshot.of("abc def", 0, 3)!!
        assertEquals("X def", (SelectionSplice.plan(head, "abc def", "X") as SelectionSplice.Plan.SetText).text)
        val tail = SelectionSnapshot.of("abc def", 4, 7)!!
        val p = SelectionSplice.plan(tail, "abc def", "X") as SelectionSplice.Plan.SetText
        assertEquals("abc X", p.text)
        assertEquals(5, p.cursor)
    }

    @Test fun surrogatePairsAroundTheSelectionSurvive() {
        val t = "😀 hello 🎉"
        val s = SelectionSnapshot.of(t, t.indexOf("hello"), t.indexOf("hello") + 5)!!
        assertEquals("😀 Hi 🎉", (SelectionSplice.plan(s, t, "Hi") as SelectionSplice.Plan.SetText).text)
    }

    @Test fun textEditedOutsideTheSelectionStillSplicesWhereTheSelectionNowIs() {
        val now = "Hello team, all. $sel Thanks!"
        val p = SelectionSplice.plan(snap, now, "Moved to Friday.") as SelectionSplice.Plan.SetText
        assertEquals("Hello team, all. Moved to Friday. Thanks!", p.text)
    }

    @Test fun aChangedOrAmbiguousSelectionIsStaleAndNothingIsTouched() {
        assertEquals(SelectionSplice.Plan.Stale, SelectionSplice.plan(snap, "something else entirely", "X"))
        assertEquals(SelectionSplice.Plan.Stale, SelectionSplice.plan(snap, null, "X"))
        val twice = SelectionSnapshot.of("ab ab", 0, 2)!!
        assertEquals("unchanged field: the captured range", "X ab", (SelectionSplice.plan(twice, "ab ab", "X") as SelectionSplice.Plan.SetText).text)
        assertEquals("changed field, two candidates: refuse", SelectionSplice.Plan.Stale, SelectionSplice.plan(twice, "ab ab ab", "X"))
    }

    @Test fun anEmptySelectionHasNothingToSplice() {
        val caret = SelectionSnapshot.of("abc", 1, 1)!!
        assertEquals(SelectionSplice.Plan.Stale, SelectionSplice.plan(caret, "abc", "X"))
    }

    @Test fun verifiesTheFieldAfterTheWrite() {
        val p = SelectionSplice.plan(snap, text, "X") as SelectionSplice.Plan.SetText
        assertTrue(SelectionSplice.applied(p, p.text))
        assertTrue("trailing whitespace an editor trims", SelectionSplice.applied(p, p.text + "\n"))
        assertFalse(SelectionSplice.applied(p, text))
        assertFalse(SelectionSplice.applied(p, null))
    }
}
