package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ScreenWordsTest {
    private class N(
        override val text: String? = null,
        override val description: String? = null,
        override val isProtected: Boolean = false,
        override val isVisible: Boolean = true,
        override val isFocusedField: Boolean = false,
        val kids: List<N> = emptyList(),
    ) : ScreenWords.Node {
        override val childCount get() = kids.size
        override fun child(i: Int): ScreenWords.Node? = kids[i]
    }

    private val chat = N(
        kids = listOf(
            N(text = "Quillon Varga", description = "Contact photo"),
            N(text = "Are we still meeting at the Brightwater office?"),
            N(text = "quillon varga"),
            N(text = "Hunter2Secret", isProtected = true),
            N(text = "Card details", isProtected = true, kids = listOf(N(text = "4000 1234 5678 9010"))),
            N(text = "Hidden draft", isVisible = false),
            N(text = "Sure, see you at", isFocusedField = true),
            N(description = "  Send\n message "),
            N(text = "+"),
        ),
    )

    @Test fun toggleOnReadsTheScreenToggleOffDoesNot() {
        assertTrue(ScreenWords.allowed(true, "com.example.chat", fieldProtected = false, userDenylist = emptySet()))
        assertFalse(ScreenWords.allowed(false, "com.example.chat", fieldProtected = false, userDenylist = emptySet()))
    }

    @Test fun protectedFieldsAndAppsAreNeverRead() {
        assertFalse(ScreenWords.allowed(true, "com.example.chat", fieldProtected = true, userDenylist = emptySet()))
        assertFalse(ScreenWords.allowed(true, "com.example.chat", fieldProtected = false, userDenylist = setOf("com.example.chat")))
    }

    @Test fun collectsVisibleTextDedupedWithoutProtectedOrFocusedNodes() {
        assertEquals(
            listOf("Quillon Varga", "Contact photo", "Are we still meeting at the Brightwater office?", "Send message"),
            ScreenWords.collect(chat),
        )
    }

    @Test fun skipsTextEqualToTheFocusedField() {
        val root = N(kids = listOf(N(text = "Draft reply"), N(text = "Header")))
        assertEquals(listOf("Header"), ScreenWords.collect(root, fieldText = " draft  reply"))
    }

    @Test fun boundedTo2000CharsAnd200PerEntry() {
        val root = N(kids = (1..100).map { N(text = "Entry $it " + "x".repeat(300)) })
        val words = ScreenWords.collect(root)
        assertTrue(words.all { it.length <= ScreenWords.MAX_ITEM })
        assertTrue(words.sumOf { it.length } <= ScreenWords.MAX_CHARS)
        assertEquals(ScreenWords.MAX_CHARS / ScreenWords.MAX_ITEM, words.size)
    }

    @Test fun walkStopsAtTheDepthAndNodeBudget() {
        var deep = N(text = "Bottom")
        repeat(ScreenWords.MAX_DEPTH + 5) { deep = N(kids = listOf(deep)) }
        assertEquals(emptyList<String>(), ScreenWords.collect(deep))

        val wide = N(kids = (1..ScreenWords.MAX_NODES * 2).map { N(text = "W$it") })
        assertEquals(ScreenWords.MAX_NODES - 1, ScreenWords.collect(wide).size) // the root counts as one
    }

    @Test fun nothingWithoutARoot() {
        assertEquals(emptyList<String>(), ScreenWords.collect(null))
    }
}
