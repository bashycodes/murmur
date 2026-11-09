package dev.flowclone.ui.tabs

import android.app.Activity
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.data.AppStyles
import dev.flowclone.data.Settings
import dev.flowclone.data.ShellPages
import dev.flowclone.data.WritingStyle
import dev.flowclone.policy.AppCategorizer.Category
import dev.flowclone.ui.AppStylesPage
import dev.flowclone.ui.DevUi
import dev.flowclone.ui.LwDrawables
import dev.flowclone.ui.ShellTab
import dev.flowclone.ui.TypeScale

/**
 * Style tab (AndroidStyle.dc.html): pill tabs per group (Personal / Work / Email / Other), the
 * apps on this phone whose dictation gets that group's style (overlapping icons; "+" opens the
 * Style-per-app "Add app" picker, [AppStylesPage]), and one card per writing style with its short
 * rule and an example bubble. Tapping a card saves it for the group at once ([Settings.styles]).
 *
 * Not drawn: the artboard's English-only footer note (no such limit in
 * the core). Per-app moves beyond "+" stay in Settings → Writing style → Style per app.
 */
internal class StyleTab(private val activity: Activity, private val ui: DevUi) : ShellTab {
    private val app get() = FlowApp.of(activity)
    override val title = ShellPages.APP_NAME

    /** The selected group. */
    var category: Category = Category.PERSONAL
        private set

    private val apps = AppStylesPage(activity, ui) { f -> update(f) }.also { it.onLoaded = { renderApps() } }
    private var view: View? = null
    private lateinit var tabs: List<TextView>
    private lateinit var appRow: LinearLayout
    private lateinit var caption: TextView
    private lateinit var cards: LinearLayout

    override fun createView(): View = view ?: ui.scrollColumn(ui.t.bg, topDp = 8) {
        tabs = tabBar(this)
        appRow = LinearLayout(activity).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        addView(appRow, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = ui.dp(18) })
        caption = ui.tv("", TypeScale.ROW, ui.t.ink2)
        addView(caption, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = ui.dp(10) })
        cards = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
        addView(cards, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = ui.dp(6) })
        render()
    }.first.also { view = it }

    /** Settings may have changed elsewhere (Settings → Writing style): show the stored state. */
    override fun onShow() {
        if (view == null) createView()
        apps.load()
        render()
    }

    /** Shows [c]'s apps and styles (a tab tap). */
    fun select(c: Category) {
        if (c == category) return
        category = c
        render()
    }

    private fun update(f: Settings.() -> Settings) {
        val old = app.settings.load()
        val n = old.f()
        if (n != old) app.settings.save(n)
        render()
    }

    private fun render() {
        if (!::tabs.isInitialized) return
        AppStyles.GROUPS.forEachIndexed { i, (c, _) -> styleTab(tabs[i], c == category) }
        caption.text = AppStyles.caption(category)
        renderApps()
        renderCards()
    }

    // ---- group tabs: 48 dp pill, 2 dp outline, the selected one filled ----

    private fun tabBar(parent: LinearLayout): List<TextView> {
        val bar = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL
            background = GradientDrawable().apply { cornerRadius = ui.dp(24).toFloat(); setStroke(ui.dp(2), ui.t.selected) }
            clipToOutline = true
            dividerDrawable = GradientDrawable().apply { setColor(ui.t.selected); setSize(ui.dp(2), 1) }
            showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
            setPadding(ui.dp(2), ui.dp(2), ui.dp(2), ui.dp(2))
        }
        val out = AppStyles.GROUPS.map { (c, label) ->
            ui.tv(label, 16f, ui.t.ink).apply {
                gravity = Gravity.CENTER
                maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END
                minHeight = ui.dp(48); minimumHeight = ui.dp(48)
                isClickable = true; isFocusable = true
                setOnClickListener { select(c) }
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.className = android.widget.RadioButton::class.java.name
                        info.isCheckable = true; info.isChecked = c == category; info.isSelected = c == category
                    }
                }
            }.also { bar.addView(it, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)) }
        }
        parent.addView(bar, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        return out
    }

    private fun styleTab(v: TextView, on: Boolean) {
        v.isSelected = on
        v.background = if (on) GradientDrawable().apply { setColor(ui.t.selected) } else ui.ripple(null)
        with(ui) { v.sans(if (on) TypeScale.W_SEMIBOLD else TypeScale.W_REGULAR) }
    }

    // ---- apps in the group: overlapping 50 dp circles, then "+" ----

    private fun renderApps() {
        if (!::appRow.isInitialized) return
        appRow.removeAllViews()
        val all = apps.apps
        val list = if (all == null) emptyList() else AppStyles.inGroup(all, app.settings.load().appBuckets, category)
        val shown = list.take(MAX_ICONS)
        val icons = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            for (a in shown) addView(appCircle(a), circleLp(overlap = true))
            if (list.size > shown.size) addView(moreCircle(list.size - shown.size), circleLp(overlap = true))
            if (all != null) {
                isFocusable = true
                contentDescription = AppStyles.appsDescription(category, list, shown.size)
            }
        }
        appRow.addView(icons)
        appRow.addView(plusCircle(), circleLp(overlap = false))
    }

    private fun circleLp(overlap: Boolean) = LinearLayout.LayoutParams(ui.dp(50), ui.dp(50)).apply { if (overlap) marginEnd = -ui.dp(10) }

    /** 50 dp circle with a 3 dp ring in the page colour (the artboard's overlap separator). */
    private fun circle(fill: Int) = FrameLayout(activity).apply {
        background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(fill); setStroke(ui.dp(3), ui.t.bg) }
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private fun appCircle(a: AppStyles.App): View = circle(ui.t.selected).apply {
        val d = apps.icon(a.pkg)
        if (d != null) addView(ImageView(activity).apply {
            setImageDrawable(d)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            background = LwDrawables.circle(Color.TRANSPARENT); clipToOutline = true
        }, FrameLayout.LayoutParams(ui.dp(44), ui.dp(44), Gravity.CENTER))
        else addView(ui.tv(a.label.take(1).uppercase(), 16f, ui.t.ink, TypeScale.W_BOLD).apply {
            gravity = Gravity.CENTER; importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    }

    private fun moreCircle(n: Int): View = circle(ui.t.card).apply {
        addView(ui.tv("+$n", 14f, ui.t.ink2, TypeScale.W_SEMIBOLD).apply {
            gravity = Gravity.CENTER; importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    }

    private fun plusCircle(): View = circle(ui.t.card).apply {
        addView(ui.icon(R.drawable.ic_lw_plus, ui.t.ink2, 24), FrameLayout.LayoutParams(ui.dp(24), ui.dp(24), Gravity.CENTER))
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        isClickable = true; isFocusable = true
        minimumHeight = ui.dp(48)
        foreground = ui.ripple(null, 25)
        contentDescription = "Add an app to ${AppStyles.groupName(category)}"
        setOnClickListener { apps.openPicker(category) }
    }

    // ---- style cards ----

    private fun renderCards() {
        cards.removeAllViews()
        val b = WritingStyle.bucket(category)
        val current = b.get(app.settings.load().styles)
        for (c in WritingStyle.choicesFor(current)) cards.addView(card(c, c.style == current) { pick(b, c) },
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = ui.dp(12) })
    }

    private fun pick(b: WritingStyle.Bucket, c: WritingStyle.Choice) {
        if (b.get(app.settings.load().styles) == c.style) return
        update { copy(styles = b.set(styles, c.style)) }
        ui.announce("${AppStyles.groupName(b.category)}: ${c.name}")
    }

    /** `.a-card`: name (22 sp bold) + short rule, a check when chosen, and the example in a bubble. */
    private fun card(c: WritingStyle.Choice, on: Boolean, onClick: () -> Unit): View = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        background = if (on) ui.outlined(ui.t.card, 24, ui.t.ink).apply { setStroke(ui.dp(2), ui.t.ink) } else ui.rounded(ui.t.card, 24)
        foreground = ui.ripple(null, 24)
        setPadding(ui.dp(20), ui.dp(20), ui.dp(20), ui.dp(20))
        minimumHeight = ui.dp(48)
        isClickable = true; isFocusable = true
        isSelected = on
        setOnClickListener { onClick() }
        contentDescription = c.cardDescription
        accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                super.onInitializeAccessibilityNodeInfo(host, info)
                info.className = android.widget.RadioButton::class.java.name
                info.isCheckable = true; info.isChecked = on
            }
        }
        val head = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            addView(LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
                addView(ui.tv(c.name, TypeScale.TITLE, ui.t.ink, TypeScale.W_BOLD))
                addView(ui.tv(c.summary, TypeScale.ROW, ui.t.ink))
            }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            if (on) addView(ui.icon(R.drawable.ic_lw_check, ui.t.ink, 24))
        }
        addView(head)
        val bubble = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            background = ui.rounded(ui.t.bubble, 14)
            setPadding(ui.dp(14), ui.dp(14), ui.dp(14), ui.dp(14))
            addView(View(activity).apply { background = LwDrawables.circle(if (ui.t.night) DAWN_700 else DAWN_500) },
                LinearLayout.LayoutParams(ui.dp(34), ui.dp(34)).apply { marginEnd = ui.dp(12) })
            addView(ui.tv(c.preview, 16f, ui.t.ink), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        }
        addView(bubble, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = ui.dp(14) })
        for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
    }

    private companion object {
        /** Icons before "+N" (the artboard shows four; five still fit 390 dp with the "+"). */
        const val MAX_ICONS = 5
        /** flow-theme.css --dawn-500 (the bubble's avatar dot), --dawn-700 on dark. */
        val DAWN_500 = Color.parseColor("#f0d7ff")
        val DAWN_700 = Color.parseColor("#a26ec1")
    }
}
