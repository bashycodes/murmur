package dev.murmur.ui

import android.app.Activity
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.ui.UiTestKit.walk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * The design-system helpers in [DevUi] (docs/design/app-design): 48 dp targets and TalkBack labels
 * on every new control, the light / dark token switch, the design switch, segmented tabs,
 * progress, stats in the serif face, and the font fallback.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class KitUiTest {
    private fun activity(): Activity = Robolectric.buildActivity(Activity::class.java).setup().get()

    private class Built(val a: Activity, val ui: DevUi, val picks: MutableList<Int>, val seg: DevUi.Segmented, val progress: DevUi.Progress, val stats: DevUi.StatRow, val toggle: DevUi.ToggleRow)

    private fun build(): Built {
        val a = activity()
        val ui = DevUi(a)
        val picks = ArrayList<Int>()
        lateinit var seg: DevUi.Segmented
        lateinit var progress: DevUi.Progress
        lateinit var stats: DevUi.StatRow
        lateinit var toggle: DevUi.ToggleRow
        with(ui) {
            screen(t.bg) {
                topBar("Murmur", DevUi.TopNav.MENU, onNav = {}, actions = listOf(DevUi.TopAction(R.drawable.ic_lw_search, "Search") {}))
                heading("Today")
                searchField("Search") {}
                seg = segmented(listOf("Personal", "Work", "Email", "Other"), 0) { picks += it }
                progress = progress(0.6f, "3 of 5 done")
                callout("Turn on the Accessibility service below.", lead = "Dictation can't start yet.")
                stats = statRow(listOf("9.4K" to "words", "121" to "wpm", "6" to "day streak"))
                sectionLabel("Engines")
                card {
                    addView(LinearLayout(ctx).apply {
                        addView(rankSquare(1)); addView(routeTag(DevUi.Route.SERVER)); addView(badge("Off", DevUi.BadgeTone.WARNING))
                        addView(badge("Granted", DevUi.BadgeTone.SUCCESS)); addView(dot(t.dotOk))
                    })
                    toggle = toggleRow("Use dictionary words", "Better at names.", checked = false)
                    navRow(null, "Speech to text", "Your server, then this phone") {}
                    addView(filledButton("Turn on") {})
                    addView(outlinedButton("Import another model") {})
                    addView(outlinedButton("Grant", pill = true) {})
                }
            }
            (a.window.decorView as FrameLayout).addView(fab(R.drawable.ic_lw_plus, "Add a word") {})
        }
        return Built(a, ui, picks, seg, progress, stats, toggle)
    }

    @Test fun everyControlIsA48dpTargetWithALabel() {
        val b = build()
        UiTestKit.assertTouchTargets(b.a, "kit")
        for (v in walk(b.a.window.decorView).filter { it.isClickable && UiTestKit.visible(it) }) {
            val label = v.contentDescription ?: (v as? TextView)?.text ?: walk(v).filterIsInstance<TextView>().joinToString { it.text }
            assertTrue("${v.javaClass.simpleName} has no label", label.isNotBlank())
        }
        val a11y = UiTestKit.a11yStrings(b.a)
        for (want in listOf("Open menu", "Search", "Add a word", "9.4K words", "6 day streak")) assertTrue(want, want in a11y)
        val fab = walk(b.a.window.decorView).filterIsInstance<ImageButton>().single { it.contentDescription == "Add a word" }
        assertTrue(fab.minimumHeight >= (56 * b.a.resources.displayMetrics.density).toInt())
    }

    @Test fun segmentedTabsPickOnceAndReadAsSelectable() {
        val b = build()
        b.seg.buttons[2].performClick()
        b.seg.buttons[2].performClick()
        assertEquals(listOf(2), b.picks)
        assertEquals(2, b.seg.selected)
        b.seg.select(1)
        assertEquals(listOf(2), b.picks)
        val info = b.seg.buttons[1].createAccessibilityNodeInfo()
        assertTrue(info.isCheckable && info.isChecked)
        assertTrue(!b.seg.buttons[2].createAccessibilityNodeInfo().isChecked)
    }

    @Test fun designSwitchIs52By32AndTheRowToggles() {
        val b = build()
        val d = b.a.resources.displayMetrics.density
        assertEquals((52 * d).toInt(), b.toggle.sw.trackDrawable.intrinsicWidth)
        assertEquals((32 * d).toInt(), b.toggle.sw.trackDrawable.intrinsicHeight)
        b.toggle.row.performClick()
        assertTrue(b.toggle.isChecked)
        assertEquals(Switch::class.java.name, b.toggle.row.createAccessibilityNodeInfo().className)
    }

    @Test fun progressAndStatsUpdate() {
        val b = build()
        assertEquals("3 of 5 done", b.progress.bar.stateDescription)
        b.progress.set(1f, "5 of 5 done")
        assertEquals("5 of 5 done", b.progress.caption.text.toString())
        b.stats.set(1, "130")
        assertEquals("130 wpm", (b.stats.numbers[1].parent as View).contentDescription)
        assertSame(Fonts.serif(b.a), b.stats.numbers[0].typeface)
    }

    @Test fun lightPaletteFollowsTheAndroidTokens() {
        val b = build()
        assertEquals(Tokens.LIGHT.bgSettings, b.ui.p.bg)
        assertEquals(Tokens.LIGHT.ink, b.ui.p.text)
        assertEquals(Tokens.LIGHT.bg, (b.a.window.decorView.background as ColorDrawable).color)
    }

    @Test @Config(qualifiers = "night") fun nightModeUsesTheDarkTokens() {
        val b = build()
        assertTrue(b.ui.t.night)
        assertEquals(Tokens.DARK.bgSettings, b.ui.p.bg)
        assertEquals(Tokens.DARK.card, b.ui.p.surface)
        assertEquals(Tokens.DARK.ink, b.ui.p.text)
        assertEquals(Tokens.DARK.bg, (b.a.window.decorView.background as ColorDrawable).color)
        val filled = walk(b.a.window.decorView).filterIsInstance<Button>().single { it.text == "Turn on" }
        assertEquals(Tokens.DARK.onButton, filled.currentTextColor)
    }

    @Test fun fontsLoadOrFallBackToTheSystemFace() {
        val ctx = RuntimeEnvironment.getApplication()
        Fonts.clear()
        assertNotNull(Fonts.sans(ctx))
        assertNotNull(Fonts.serif(ctx, 600))
        // Same weight comes from the cache.
        assertSame(Fonts.sans(ctx, 550), Fonts.sans(ctx, 550))
        val missing = Fonts.get(ctx.assets, "fonts/missing.ttf", 600, Typeface.SANS_SERIF)
        assertNotNull(missing)
        // Helper text uses the sans face.
        val b = build()
        val title = walk(b.a.window.decorView).filterIsInstance<TextView>().first { it.text == "Speech to text" }
        assertSame(Fonts.sans(b.a, TypeScale.W_MEDIUM), title.typeface)
    }

    /** Status bar and cutout pad the frame (content never scrolls under the clock); the bottom bar / keyboard pad the scroll. */
    @Test fun screenKeepsContentBelowTheStatusBar() {
        val pad = ScreenPadding.of(left = 0, top = 63, right = 0, bottom = 48) { it * 2 }
        assertEquals(ScreenPadding(frameLeft = 0, frameTop = 63, frameRight = 0, scrollSide = 32, scrollTop = 16, scrollBottom = 96), pad)
        val a = activity()
        val ui = DevUi(a)
        val col = ui.screen { }
        val scroll = col.parent as android.widget.ScrollView
        assertSame(scroll, ui.currentScroll())
        assertTrue("the scroll sits inside a frame that takes the top inset", scroll.parent is FrameLayout)
    }

    /** A header (a settings page's top bar) sits above the scroll view, so it stays put while the column scrolls. */
    @Test fun screenHeaderStaysOutsideTheScroll() {
        val a = activity()
        val ui = DevUi(a)
        lateinit var head: LinearLayout
        val col = ui.screen(header = { head = this }) { }
        val scroll = col.parent as android.widget.ScrollView
        assertSame(scroll, ui.currentScroll())
        assertSame("header and scroll share one parent", scroll.parent, head.parent)
        // A rebuilt page (no header) finds its new scroll view, not the replaced one.
        val again = ui.rebuild { }
        assertSame(again.parent, ui.currentScroll())
    }

    /** A row and its chips form one card item: the hairline goes between items, not under a row's own button. */
    @Test fun cardItemKeepsARowWithItsChips() {
        val a = activity()
        val ui = DevUi(a)
        lateinit var card: LinearLayout
        with(ui) {
            screen {
                card = card {
                    for (n in listOf("Model A", "Model B")) item {
                        stateRow(null, n, "490 MB", "", Tone.NEUTRAL)
                        chipRow(chip("Download") {})
                    }
                }
            }
        }
        assertEquals(2, card.childCount)
        for (i in 0 until card.childCount) assertEquals(2, (card.getChildAt(i) as LinearLayout).childCount)
    }

    /** Notes take the column's full width so a wrapped last line is never clipped at the end edge. */
    @Test fun noteFillsTheColumnWidth() {
        val a = activity()
        lateinit var note: TextView
        with(DevUi(a)) { screen { note = note("Optional. A long explanation that wraps over several lines on a phone.") } }
        assertEquals(android.view.ViewGroup.LayoutParams.MATCH_PARENT, note.layoutParams.width)
    }
}
