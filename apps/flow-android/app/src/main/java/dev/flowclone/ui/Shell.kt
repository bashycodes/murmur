package dev.flowclone.ui

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import dev.flowclone.R
import dev.flowclone.data.ShellPages.Tab

/**
 * One bottom-navigation destination of the app shell ([MainActivity]).
 *
 * Extension point for the tab chunks: [MainActivity.createTab] maps each [Tab] to a ShellTab.
 * An *embedded* tab returns its content from [createView] (the shell puts it between the top bar
 * and the bottom nav, so the nav stays visible) and gets [onShow] / [onHide] when it is selected /
 * deselected and when the activity resumes / pauses. A *launch* tab ([LaunchTab]) returns an
 * Intent from [launchIntent] instead: selecting it starts that activity and the nav keeps the
 * previous tab selected (the bridge for screens not yet restyled into a tab).
 */
internal interface ShellTab {
    /** Top-bar title while this tab shows (Home: "Murmur"). */
    val title: String

    /** Top-bar actions while this tab shows (e.g. Add). */
    fun actions(): List<DevUi.TopAction> = emptyList()

    /** Non-null: this tab is a separate activity for now ([LaunchTab]); [createView] is not called. */
    fun launchIntent(): Intent? = null

    /** The tab's content, created once per activity instance. */
    fun createView(): View = throw UnsupportedOperationException("launch tab")

    /** Selected, or the activity resumed while it is selected: reload what may have changed. */
    fun onShow() {}

    /** Deselected, or the activity paused. */
    fun onHide() {}
}

/** A tab that still opens its pre-design activity (Dictionary / Style / Snippets until their chunks land). */
internal class LaunchTab(override val title: String, private val intent: () -> Intent) : ShellTab {
    override fun launchIntent(): Intent = intent()
}

/**
 * The shell's views (AndroidHome / AndroidNav / AndroidDrawer.dc.html): a 64 dp top bar, the tab
 * host, the 76 dp bottom navigation, and the drawer over a scrim. Pads itself by the system bars.
 */
internal class ShellFrame(private val activity: Activity, private val ui: DevUi, private val onPick: (Tab) -> Unit) {
    private val t get() = ui.t
    private fun dp(v: Int) = ui.dp(v)

    val root = FrameLayout(activity)
    private val main = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private val topBarBox = FrameLayout(activity)
    /** Where the selected embedded tab's view goes. */
    val host = FrameLayout(activity)
    val nav = LinearLayout(activity).apply { orientation = LinearLayout.HORIZONTAL }
    private val navItems = LinkedHashMap<Tab, NavItem>()
    val scrim = View(activity)
    /** The drawer panel; [drawerContent] holds what [MainActivity] puts in it. */
    val drawer = FrameLayout(activity)
    val drawerContent = FrameLayout(activity)
    var topBar: DevUi.TopBar? = null; private set

    var onDrawerChanged: ((Boolean) -> Unit)? = null

    private class NavItem(val view: LinearLayout, val pill: FrameLayout, val icon: android.widget.ImageView, val label: TextView)

    init {
        root.setBackgroundColor(t.bg)
        main.addView(topBarBox, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)))
        main.addView(host, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        nav.setPadding(0, dp(8), 0, dp(10))
        nav.setBackgroundColor(t.bg)
        nav.accessibilityPaneTitle = "Main"
        for (tab in Tab.entries) nav.addView(navItem(tab), LinearLayout.LayoutParams(0, dp(58), 1f))
        main.addView(nav, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        root.addView(main, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        // Scrim: tap (or TalkBack "Close menu") closes the drawer.
        scrim.setBackgroundColor(Color.argb(0x52, 0x1A, 0x1A, 0x1A))
        scrim.contentDescription = "Close menu"
        scrim.isClickable = true
        scrim.setOnClickListener { closeDrawer() }
        scrim.visibility = View.GONE
        root.addView(scrim, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        drawer.background = GradientDrawable().apply {
            setColor(t.bgSettings)
            val r = dp(24).toFloat()
            cornerRadii = floatArrayOf(0f, 0f, r, r, r, r, 0f, 0f)
        }
        drawer.clipToOutline = true
        drawer.elevation = dp(2).toFloat()
        drawer.accessibilityPaneTitle = "Menu"
        swallowTouches(drawer)
        drawer.visibility = View.GONE
        drawer.addView(drawerContent, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val width = minOf(dp(310), (activity.resources.displayMetrics.widthPixels * 0.85f).toInt().coerceAtLeast(dp(280)))
        root.addView(drawer, FrameLayout.LayoutParams(width, ViewGroup.LayoutParams.MATCH_PARENT, Gravity.START))

        root.setOnApplyWindowInsetsListener { _, insets ->
            val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            main.setPadding(i.left, i.top, i.right, i.bottom)
            drawerContent.setPadding(i.left, i.top, 0, i.bottom)
            insets
        }
    }

    /**
     * Taps on the panel's empty parts must not fall through to the scrim (which closes it). Not
     * isClickable: the panel is not a button for TalkBack, and its rows stay the clickable views.
     */
    @android.annotation.SuppressLint("ClickableViewAccessibility")
    private fun swallowTouches(v: View) = v.setOnTouchListener { _, _ -> true }

    private fun navItem(tab: Tab): View {
        val icon = ui.icon(NAV_ICONS.getValue(tab), t.ink2, 24)
        val pill = FrameLayout(activity).apply {
            addView(icon, FrameLayout.LayoutParams(dp(24), dp(24), Gravity.CENTER))
        }
        val label = ui.tv(tab.label, 14f, t.ink2, 500).apply { gravity = Gravity.CENTER; maxLines = 1 }
        val item = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            addView(pill, LinearLayout.LayoutParams(dp(64), dp(32)).apply { bottomMargin = dp(4) })
            addView(label, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            minimumHeight = dp(48)
            isClickable = true; isFocusable = true
            background = ui.ripple(null, 24)
            contentDescription = tab.label
            setOnClickListener { onPick(tab) }
            accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.isSelected = host.isSelected
                }
            }
        }
        navItems[tab] = NavItem(item, pill, icon, label)
        return item
    }

    /** Highlights [tab] in the bottom nav (pill, ink colour, semibold). */
    fun select(tab: Tab) {
        for ((k, n) in navItems) {
            val on = k == tab
            n.view.isSelected = on
            n.pill.background = if (on) ui.rounded(t.selected, 16) else null
            n.icon.imageTintList = android.content.res.ColorStateList.valueOf(if (on) t.ink else t.ink2)
            n.label.setTextColor(if (on) t.ink else t.ink2)
            n.label.typeface = Fonts.sans(activity, if (on) 600 else 500)
        }
    }

    /** The bottom-nav item of [tab] (tests, focus). */
    fun navView(tab: Tab): View = navItems.getValue(tab).view

    fun setTopBar(title: String, actions: List<DevUi.TopAction>) {
        topBarBox.removeAllViews()
        val bar = ui.buildTopBar(title, DevUi.TopNav.MENU, { openDrawer() }, actions, mainTab = true)
        topBarBox.addView(bar.view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)))
        topBar = bar
    }

    fun setContent(v: View) {
        host.removeAllViews()
        (v.parent as? ViewGroup)?.removeView(v)
        host.addView(v, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    }

    val drawerOpen: Boolean get() = drawer.visibility == View.VISIBLE

    fun openDrawer() {
        if (drawerOpen) return
        scrim.visibility = View.VISIBLE
        drawer.visibility = View.VISIBLE
        drawer.translationX = -dp(310).toFloat()
        drawer.animate().translationX(0f).setDuration(200).start()
        scrim.alpha = 0f
        scrim.animate().alpha(1f).setDuration(200).start()
        // TalkBack: only the drawer while it is open, and focus moves into it.
        main.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        drawer.sendAccessibilityEvent(AccessibilityEvent.TYPE_VIEW_FOCUSED)
        onDrawerChanged?.invoke(true)
    }

    fun closeDrawer() {
        if (!drawerOpen) return
        drawer.animate().cancel(); scrim.animate().cancel()
        drawer.translationX = 0f
        drawer.visibility = View.GONE
        scrim.visibility = View.GONE
        main.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
        topBar?.nav?.sendAccessibilityEvent(AccessibilityEvent.TYPE_VIEW_FOCUSED)
        onDrawerChanged?.invoke(false)
    }

    companion object {
        val NAV_ICONS = mapOf(
            Tab.HOME to R.drawable.ic_lw_nav_home, Tab.DICTIONARY to R.drawable.ic_lw_nav_dictionary,
            Tab.STYLE to R.drawable.ic_lw_nav_style, Tab.SNIPPETS to R.drawable.ic_lw_nav_snippets,
        )
    }
}
