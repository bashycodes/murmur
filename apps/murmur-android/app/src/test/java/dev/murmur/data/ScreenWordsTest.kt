package dev.murmur.data

import dev.murmur.core.FieldContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ScreenWordsTest {
    private open class N(
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
    private val expected = listOf("Quillon Varga", "Contact photo", "Are we still meeting at the Brightwater office?", "Send message")

    private fun gate(enabled: Boolean = true, pkg: String? = "com.example.chat", fieldProtected: Boolean = false, deny: Set<String> = emptySet()) =
        ScreenWords.Gate(enabled, pkg, fieldProtected, deny)

    private val base = FieldContext(appId = "com.example.chat", before = "Sure, see you at ")

    // ---- the wiring: the toggle decides whether the context carries words ----

    @Test fun toggleOnPutsTheWordsInTheContext() {
        val ctx = ScreenWords.attach(base, gate(), { chat }, nodeText = null, imeText = null)
        assertEquals(expected, ctx.screenWords)
        assertEquals(base.before, ctx.before)
    }

    @Test fun toggleOffLeavesNoWordsAndNeverReadsTheScreen() {
        var read = false
        val ctx = ScreenWords.attach(base, gate(enabled = false), { read = true; chat }, nodeText = null, imeText = null)
        assertEquals(emptyList<String>(), ctx.screenWords)
        assertFalse(read)
    }

    @Test fun protectedFieldsDenylistedBankingAndUnknownAppsGetNoWords() {
        for (g in listOf(gate(fieldProtected = true), gate(deny = setOf("com.example.chat")), gate(pkg = null))) {
            assertFalse(ScreenWords.allowed(g))
            assertEquals(emptyList<String>(), ScreenWords.attach(base, g, { chat }, null, null).screenWords)
        }
        assertTrue(ScreenWords.allowed(gate()))
    }

    @Test fun theFocusedFieldsFullTextIsLeftOut() {
        val root = N(kids = listOf(N(text = "Lunch on Friday? Bring the maps."), N(text = "Header")))
        val full = "Lunch on Friday? Bring the maps."
        // The node's whole text wins over the IME's window (which may start mid-text).
        assertEquals(listOf("Header"), ScreenWords.attach(base, gate(), { root }, nodeText = full, imeText = "Bring the maps.").screenWords)
        // No node: the IME's text.
        assertEquals(listOf("Header"), ScreenWords.attach(base, gate(), { root }, nodeText = null, imeText = " lunch on  friday? bring the maps.").screenWords)
    }

    // ---- the walk ----

    @Test fun collectsVisibleTextDedupedWithoutProtectedOrFocusedNodes() {
        assertEquals(expected, ScreenWords.collect(chat))
    }

    @Test fun boundedTo2000CharsAnd200PerEntry() {
        val root = N(kids = (1..100).map { N(text = "Entry $it " + "x".repeat(300)) })
        val words = ScreenWords.collect(root)
        assertTrue(words.all { it.length <= ScreenWords.MAX_ENTRY_CHARS })
        assertTrue(words.sumOf { it.length } <= ScreenWords.MAX_CHARS)
        assertEquals(ScreenWords.MAX_CHARS / ScreenWords.MAX_ENTRY_CHARS, words.size)
    }

    @Test fun hebrewStaysInsideTheServerContextLimits() {
        // murmur-server rejects a context over 1000 entries or 64 000 bytes of words (limits::context).
        val letters = ('א'..'ת').toList()
        val root = N(kids = (0 until 3000).map { i -> N(text = "${letters[i % letters.size]}${letters[(i / letters.size) % letters.size]}${i}") })
        val words = ScreenWords.collect(root, deadline = Long.MAX_VALUE)
        assertTrue(words.size <= ScreenWords.MAX_ENTRIES)
        assertTrue(words.sumOf { it.toByteArray(Charsets.UTF_8).size } <= 64_000)
        val one = N(kids = listOf(N(text = "ש".repeat(5000))))
        assertTrue(ScreenWords.collect(one).sumOf { it.toByteArray(Charsets.UTF_8).size } <= 2 * ScreenWords.MAX_CHARS)
    }

    @Test fun walkStopsAtTheDepthAndNodeBudget() {
        var deep = N(text = "Bottom")
        repeat(ScreenWords.MAX_DEPTH + 5) { deep = N(kids = listOf(deep)) }
        assertEquals(emptyList<String>(), ScreenWords.collect(deep))

        var visited = 0
        val wide = N(kids = (1..ScreenWords.MAX_NODES * 2).map { object : N(text = "W$it") { override val isVisible get() = true.also { visited++ } } })
        // Text-free nodes would not stop at MAX_ENTRIES; these do, well before MAX_NODES.
        assertEquals(ScreenWords.MAX_ENTRIES, ScreenWords.collect(wide, deadline = Long.MAX_VALUE).size)
        visited = 0
        val blank = N(kids = (1..ScreenWords.MAX_NODES * 2).map { object : N() { override val isVisible get() = true.also { visited++ } } })
        ScreenWords.collect(blank, deadline = Long.MAX_VALUE)
        assertEquals(ScreenWords.MAX_NODES - 1, visited) // the root counts as one
    }

    @Test fun deadlineStopsTheWalkAndKeepsWhatItHas() {
        var clock = 0L
        val slow = N(kids = (1..50).map { i -> object : N(text = "Item $i") { override val text get() = super.text.also { clock += 10 } } })
        val words = ScreenWords.collect(slow, deadline = 30, now = { clock })
        assertEquals(listOf("Item 1", "Item 2", "Item 3"), words)
    }

    @Test fun attachUsesA30MsBudget() {
        var clock = 1_000L
        val slow = N(kids = (1..50).map { i -> object : N(text = "Item $i") { override val text get() = super.text.also { clock += 10 } } })
        val ctx = ScreenWords.attach(base, gate(), { slow }, null, null, now = { clock })
        assertEquals(3, ctx.screenWords.size)
    }

    @Test fun nothingWithoutARoot() {
        assertEquals(emptyList<String>(), ScreenWords.collect(null))
        assertEquals(emptyList<String>(), ScreenWords.attach(base, gate(), { null }, null, null).screenWords)
    }
}
