package dev.flowclone.ui

import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.PopupMenu
import android.widget.ScrollView
import dev.flowclone.R
import dev.flowclone.data.ShellPages

/**
 * Parts the secondary screens (History, Notes, Insights, note editor) share on top of [DevUi]:
 * the page with a floating action button, the card action row (Copy pill + ⋮ menu) of
 * AndroidHome.dc.html's dictation cards, and TalkBack custom actions on a card.
 */

/** Route tag for a dictation's [ShellPages.RouteKind]; null = unknown (no tag). */
internal fun routeOf(k: ShellPages.RouteKind?): DevUi.Route? = when (k) {
    ShellPages.RouteKind.PHONE -> DevUi.Route.PHONE
    ShellPages.RouteKind.SERVER -> DevUi.Route.SERVER
    ShellPages.RouteKind.CLOUD -> DevUi.Route.CLOUD
    null -> null
}

/** A page with a floating action button: what [DevUi.screen] attaches, plus the FAB. */
internal class FabPage(val column: LinearLayout, val scroll: ScrollView, val fab: ImageButton)

/**
 * Attaches a scrolling column on [background] with `.a-fab` ([DevUi.fab]) floating bottom-right;
 * the whole frame pads itself by the system bars, cutout and keyboard, and the column leaves room
 * under its last card so the FAB never covers it.
 */
internal fun DevUi.fabPage(fabIcon: Int, fabDescription: String, onFab: () -> Unit, background: Int = p.bg, build: LinearLayout.() -> Unit): FabPage {
    val (scroll, col) = scrollColumn(background, topDp = 8, build = build)
    col.setPadding(col.paddingLeft, col.paddingTop, col.paddingRight, dp(96))
    val fab = fab(fabIcon, fabDescription, onFab)
    val frame = FrameLayout(ctx).apply {
        setBackgroundColor(background)
        addView(scroll, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        addView(fab)
        setOnApplyWindowInsetsListener { v, insets ->
            val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
            v.setPadding(i.left, i.top, i.right, i.bottom)
            insets
        }
    }
    activity.window.decorView.setBackgroundColor(background)
    activity.setContentView(frame)
    frame.requestApplyInsets()
    return FabPage(col, scroll, fab)
}

/** Offers [actions] to TalkBack as custom actions on [v]; long-press runs the first one. */
internal fun customActions(v: View, actions: List<Pair<String, () -> Unit>>) {
    if (actions.isEmpty()) return
    v.setOnLongClickListener { actions.first().second(); true }
    val ids = actions.map { View.generateViewId() }
    v.accessibilityDelegate = object : View.AccessibilityDelegate() {
        override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
            super.onInitializeAccessibilityNodeInfo(host, info)
            actions.forEachIndexed { i, a -> info.addAction(AccessibilityNodeInfo.AccessibilityAction(ids[i], a.first)) }
        }
        override fun performAccessibilityAction(host: View, action: Int, args: android.os.Bundle?): Boolean {
            val i = ids.indexOf(action)
            if (i >= 0) { actions[i].second(); return true }
            return super.performAccessibilityAction(host, action, args)
        }
    }
}

/** The design's in-card "Copy" pill: copy icon + label, 48 dp tall. */
internal fun DevUi.copyPill(description: String, onClick: () -> Unit): LinearLayout = LinearLayout(ctx).apply {
    orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
    minimumHeight = dp(48)
    setPadding(dp(12), 0, dp(12), 0)
    background = ripple(null, 24)
    isClickable = true; isFocusable = true
    contentDescription = description
    addView(icon(R.drawable.ic_lw_copy, p.text, 18).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(8) })
    addView(tv("Copy", 15f, p.text, TypeScale.W_MEDIUM))
    setOnClickListener { onClick() }
}

/** ⋮ button that opens a popup with [items]. */
internal fun DevUi.moreButton(description: String, items: List<Pair<String, () -> Unit>>): ImageButton {
    lateinit var b: ImageButton
    b = iconButton(R.drawable.ic_lw_more, description) {
        PopupMenu(ctx, b).apply {
            items.forEachIndexed { i, (label, _) -> menu.add(0, i, i, label) }
            setOnMenuItemClickListener { items[it.itemId].second(); true }
        }.show()
    }
    return b
}

/**
 * The bottom row of a card: [start] (usually [copyPill]) on the left, [end] (usually
 * [moreButton]) on the right; pulled 8 dp into the card's side padding like the design.
 */
internal fun DevUi.cardActions(start: View?, end: View?): LinearLayout = LinearLayout(ctx).apply {
    orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
    if (start != null) addView(start, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    addView(View(ctx), LinearLayout.LayoutParams(0, 1, 1f))
    if (end != null) addView(end)
    layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
        topMargin = dp(6); marginStart = -dp(8); marginEnd = -dp(8)
    }
}

/** A standalone `.a-card` (24 dp, card colour) holding one item; the design's dictation / note card. */
internal fun DevUi.itemCard(): LinearLayout = LinearLayout(ctx).apply {
    orientation = LinearLayout.VERTICAL
    background = rounded(p.surface, 24)
    setPadding(dp(20), dp(18), dp(20), dp(8))
    layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6); bottomMargin = dp(6) }
}
