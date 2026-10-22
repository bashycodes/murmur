package dev.flowclone.insert

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TextMatchingTest {
    @Test fun containsTrimmedText() = assertTrue(TextMatching.containsCommittedText("hello world.", " world. "))

    @Test fun matchIsCaseSensitive() = assertFalse(TextMatching.containsCommittedText("Hello", "hello"))

    @Test fun longTextAcceptsTrailing400Chars() {
        val text = "a".repeat(200) + "b".repeat(400)
        assertTrue(TextMatching.containsCommittedText("x" + "b".repeat(400), text))
    }

    @Test fun longTextAcceptsLeading400Chars() {
        val text = "c".repeat(400) + "d".repeat(200)
        assertTrue(TextMatching.containsCommittedText("c".repeat(400) + "zzz", text))
    }

    @Test fun shortTextNeedsFullMatch() = assertFalse(TextMatching.containsCommittedText("hello", "hello there"))

    @Test fun composeAlreadyPresent() =
        assertEquals(TextMatching.Compose.AlreadyPresent, TextMatching.compose("hi there", "say hi there"))

    @Test fun composeRefusesUnreadable() = assertEquals(TextMatching.Compose.RefusedUnreadable, TextMatching.compose("hi", null))

    @Test fun composeWritesIntoEmpty() = assertEquals(TextMatching.Compose.Write, TextMatching.compose("hi", ""))

    @Test fun composeNeverOverwritesNonEmpty() =
        assertEquals(TextMatching.Compose.RefusedNonEmpty, TextMatching.compose("hi", "other text"))

    @Test fun normalizePlaceholderStripsEllipsisAndCase() =
        assertEquals("type a message", TextMatching.normalizePlaceholder("  Type a message… "))

    @Test fun nodeShowingHintReadsEmpty() =
        assertEquals("", TextMatching.readNodeText(NodeSnapshot(text = "Search", showingHint = true), null, emptySet()))

    @Test fun declaredHintMatchReadsEmpty() =
        assertEquals("", TextMatching.readNodeText(NodeSnapshot(text = "Write a note...", hint = "write a note"), null, emptySet()))

    @Test fun declaredHintMismatchKeepsText() =
        assertEquals("real", TextMatching.readNodeText(NodeSnapshot(text = "real", hint = "write a note"), "real", emptySet()))

    @Test fun editorHintMatchReadsEmpty() =
        assertEquals("", TextMatching.readNodeText(NodeSnapshot(text = "Reply"), "Reply", emptySet()))

    @Test fun knownListOnlyWithoutEditorHint() {
        val known = setOf("message")
        assertEquals("", TextMatching.readNodeText(NodeSnapshot(text = "Message"), null, known))
        assertEquals("Message", TextMatching.readNodeText(NodeSnapshot(text = "Message"), "Other hint", known))
    }

    /** Review P3: text equal to the contentDescription is user text, never overwritten. */
    @Test fun contentDescriptionNeverMakesTextAPlaceholder() {
        assertEquals("Comment", TextMatching.readNodeText(NodeSnapshot(text = "Comment", contentDescription = "comment"), null, emptySet()))
        assertEquals(TextMatching.Compose.RefusedNonEmpty, TextMatching.compose("hi", TextMatching.readNodeText(NodeSnapshot(text = "ok thanks", contentDescription = "ok thanks"), null, emptySet())))
        val long = "x".repeat(49)
        assertEquals(long, TextMatching.readNodeText(NodeSnapshot(text = long, contentDescription = long), null, emptySet()))
    }

    @Test fun nullNodeTextIsUnreadable() = assertNull(TextMatching.readNodeText(NodeSnapshot(text = null), null, emptySet()))

    @Test fun knownComposeHintsOnlyForWhatsappWhenStaleProbed() {
        assertTrue(TextMatching.knownComposeHints("com.whatsapp", staleProbed = true).contains("message"))
        assertTrue(TextMatching.knownComposeHints("com.whatsapp", staleProbed = false).isEmpty())
        assertTrue(TextMatching.knownComposeHints("org.telegram.messenger", staleProbed = true).isEmpty())
    }
}
