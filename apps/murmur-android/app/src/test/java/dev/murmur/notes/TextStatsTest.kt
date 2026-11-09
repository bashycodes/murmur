package dev.murmur.notes

import org.junit.Assert.assertEquals
import org.junit.Test

class TextStatsTest {
    @Test fun countsWordsAcrossLinesAndPunctuation() {
        assertEquals(0, TextStats.words(""))
        assertEquals(0, TextStats.words("  \n\t "))
        assertEquals(0, TextStats.words(" - — … "))
        assertEquals(1, TextStats.words("hello"))
        assertEquals(5, TextStats.words("Hello, world.\nThis is   fine!"))
        assertEquals(3, TextStats.words("don't re-read 42"))
        assertEquals(2, TextStats.words("שלום עולם"))
    }

    @Test fun labelPluralises() {
        assertEquals("0 words", TextStats.label(""))
        assertEquals("1 word", TextStats.label("one"))
        assertEquals("2 words", TextStats.label("one two"))
    }
}
