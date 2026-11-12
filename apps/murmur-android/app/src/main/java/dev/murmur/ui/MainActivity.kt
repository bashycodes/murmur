package dev.murmur.ui

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.murmur.EngineHolder
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.NativeMurmurCore
import dev.murmur.data.AppVersion
import dev.murmur.data.SettingsPages
import dev.murmur.data.SetupChecklist
import dev.murmur.data.ShellPages
import dev.murmur.data.ShellPages.Extra
import dev.murmur.data.ShellPages.Tab
import dev.murmur.data.SpeechEngines
import dev.murmur.service.MurmurService
import kotlin.concurrent.thread

/**
 * The app shell (launcher; AndroidHome / AndroidNav / AndroidDrawer.dc.html): top bar with the
 * menu button, the selected tab, bottom navigation (Home / Dictionary / Style / Snippets) and the
 * drawer (stats, engines, Settings / Permissions / Help, the features the design has no slot for,
 * version). Back closes the drawer first.
 *
 * Tabs: [createTab] maps each [Tab] to an embedded [ShellTab] (Home, Dictionary, Style, Snippets);
 * [LaunchTab] remains for a destination that has to be its own activity.
 */
class MainActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private lateinit var ui: DevUi
    internal lateinit var frame: ShellFrame
        private set
    private val tabs = HashMap<Tab, ShellTab>()
    internal var current = Tab.HOME
        private set
    private val main = Handler(Looper.getMainLooper())
    private var shown: String? = null
    private var resumed = false
    private var backCallback: OnBackInvokedCallback? = null

    /** Re-check setup / engine state while the shell is in front (the Home notices follow it). */
    private val refresh = object : Runnable {
        override fun run() {
            val sig = signature()
            if (sig != shown) { shown = sig; updateNotices() }
            main.postDelayed(this, 2_000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        frame = ShellFrame(this, ui, ::pick)
        frame.onDrawerChanged = ::drawerChanged
        window.decorView.setBackgroundColor(ui.t.bg)
        setContentView(frame.root)
        frame.root.requestApplyInsets()
        show(ShellPages.tabOf(savedInstanceState?.getString(STATE_TAB)).takeIf { tab(it).launchIntent() == null } ?: Tab.HOME)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString(STATE_TAB, current.name)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        tab(current).onShow()
        shown = null
        refresh.run()
        if (frame.drawerOpen) fillDrawer()
    }

    override fun onPause() {
        resumed = false
        main.removeCallbacks(refresh)
        tab(current).onHide()
        super.onPause()
    }

    override fun onDestroy() {
        backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
        super.onDestroy()
    }

    // ---- tabs ----

    /** The extension point: which [ShellTab] hosts each bottom-nav destination. */
    private fun createTab(t: Tab): ShellTab = when (t) {
        Tab.HOME -> HomeTab(this, ui)
        Tab.DICTIONARY -> dev.murmur.ui.tabs.DictionaryTab(this, ui)
        Tab.STYLE -> dev.murmur.ui.tabs.StyleTab(this, ui)
        Tab.SNIPPETS -> dev.murmur.ui.tabs.SnippetsTab(this, ui)
    }

    internal fun tab(t: Tab): ShellTab = tabs.getOrPut(t) { createTab(t) }

    private fun pick(t: Tab) {
        val st = tab(t)
        st.launchIntent()?.let { startActivity(it); return }
        if (t == current) return
        if (resumed) tab(current).onHide()
        show(t)
        if (resumed) st.onShow()
    }

    private fun show(t: Tab) {
        current = t
        val st = tab(t)
        frame.select(t)
        frame.setTopBar(st.title, st.actions())
        frame.setContent(st.createView())
        if (t == Tab.HOME) updateNotices()
    }

    // ---- Home notices ----

    private fun signature() = listOf(
        PermissionsActivity.setupState(this), app.engine.state.value, app.engine.libraryState()::class.simpleName,
    ).joinToString("|") { if (it is EngineHolder.State.Ready) "ready" else it.toString() }

    private fun updateNotices() {
        val home = tabs[Tab.HOME] as? HomeTab ?: return
        val out = ArrayList<HomeTab.Notice>()
        val setup = PermissionsActivity.setupState(this)
        SetupChecklist.banner(setup)?.takeIf { it.blocking }?.let { b ->
            val (lead, text) = ShellPages.callout(b.text)
            out += HomeTab.Notice(lead, text.replace(" below.", " in Permissions."), ::openPermissions)
        }
        when (val lib = app.engine.libraryState()) {
            is NativeMurmurCore.LibraryState.Unavailable -> out += HomeTab.Notice(
                "Speech engine unavailable.",
                "This build was packaged without libmurmur_ffi.so (${lib.message}). Dictation needs it, or turn on the mock engine in Settings.",
            ) { startActivity(SettingsActivity.intent(this)) }
            is NativeMurmurCore.LibraryState.Loaded -> NativeMurmurCore.capabilities()?.let { c ->
                // BuildConfig.CAP_* must equal nativeCapabilities() (D17 §1.4).
                if (c.local != app.caps.local || c.remote != app.caps.remote) out += HomeTab.Notice(
                    "Native library flavour mismatch.",
                    "This is the ${app.caps.flavor} APK but libmurmur_ffi.so was built with local=${c.local} remote=${c.remote}.",
                ) {}
            }
        }
        (app.engine.state.value as? EngineHolder.State.Failed)?.let { f ->
            out += HomeTab.Notice("Speech engine failed.", f.message) { openEngines() }
        }
        home.setNotices(out)
    }

    // ---- drawer ----

    private fun drawerChanged(open: Boolean) {
        if (open) {
            fillDrawer()
            if (backCallback == null) backCallback = OnBackInvokedCallback { frame.closeDrawer() }.also {
                onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, it)
            }
        } else {
            backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
            backCallback = null
        }
    }

    /** Back closes the drawer first (when predictive back is off; [backCallback] when it is on). */
    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        if (frame.drawerOpen) frame.closeDrawer() else @Suppress("DEPRECATION") super.onBackPressed()
    }

    /** Close the drawer, then go. */
    private fun go(f: () -> Unit) { frame.closeDrawer(); f() }

    private fun fillDrawer(): Unit = with(ui) {
        val setup = PermissionsActivity.setupState(this@MainActivity)
        val dev = app.dev.load()
        val settings = app.settings.load()
        val (scroll, _) = scrollColumn(null, topDp = 24) {
            addView(tv(ShellPages.APP_NAME, TypeScale.TITLE, p.text, TypeScale.W_BOLD).apply {
                letterSpacing = TypeScale.TITLE_TRACKING
                setPadding(dp(8), 0, dp(8), dp(8))
                isAccessibilityHeading = true
            })
            val stats = statRow(listOf("–" to "words", "–" to "wpm", "–" to "day streak"))
            thread(name = "drawer-stats") {
                val s = runCatching { app.insights.summary() }.getOrNull() ?: return@thread
                runOnUiThread { ShellPages.stats(s).forEachIndexed { i, (n, u) -> stats.set(i, n, u) } }
            }
            enginesCard(
                ShellPages.speechLine(dev, app.caps, SpeechEngines.phoneReady(settings, app.engine.defaultModelInstalled(settings))),
                ShellPages.cleanupLine(
                    settings, dev.effective(app.caps), app.caps,
                    serverLlm = if (dev.serverConfigured) app.engine.router()?.cleanupStatus()?.serverLlm else null,
                ),
            )
            card {
                navRow(R.drawable.ic_lw_settings, "Settings", chevron = false) { go { startActivity(SettingsActivity.intent(this@MainActivity)) } }
                // Re-read on every drawer open and on resume while open (fillDrawer), so a grant made in
                // system settings shows on return. The subtitle names what is missing.
                val rows = SetupChecklist.rows(setup)
                val perm = navRow(R.drawable.ic_lw_check, "Permissions", ShellPages.missing(rows), chevron = false) { go(::openPermissions) }
                ShellPages.toFix(rows)?.let { fix ->
                    (perm.view as LinearLayout).addView(badge(fix, DevUi.BadgeTone.WARNING))
                    perm.view.contentDescription = "Permissions, $fix: ${ShellPages.missing(rows)}"
                }
                navRow(R.drawable.ic_lw_help, "Help", chevron = false) {
                    go { runCatching { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(ShellPages.HELP_URL))) } }
                }
            }
            sectionLabel("More")
            card {
                for (e in ShellPages.extras(app.caps.local, app.dev.visible)) navRow(EXTRA_ICONS.getValue(e), e.title, e.subtitle, chevron = false) { go { extra(e) } }
            }
            note(ShellPages.versionLine(AppVersion.current.version), p.muted, TypeScale.META).apply { setPadding(dp(8), dp(8), dp(8), 0) }
        }
        frame.drawerContent.removeAllViews()
        frame.drawerContent.addView(scroll, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    }

    /** "ENGINES" card: speech + cleanup lines with their status dots; each line opens its own page ([ShellPages.EngineLine.dest]). */
    private fun LinearLayout.enginesCard(vararg lines: ShellPages.EngineLine): Unit = with(ui) {
        val box = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            background = rounded(p.surface, 24)
            setPadding(dp(8), dp(16), dp(8), dp(8))
            addView(tv("Engines", TypeScale.SECTION, p.muted, TypeScale.W_SEMIBOLD).apply {
                isAllCaps = true; letterSpacing = TypeScale.SECTION_TRACKING
                setPadding(dp(12), 0, dp(12), 0)
                isAccessibilityHeading = true
            })
            for (l in lines) addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                background = ripple(rounded(p.surface, 16), 16)
                setPadding(dp(12), dp(8), dp(12), dp(8))
                isClickable = true; isFocusable = true
                minimumHeight = dp(48)
                setOnClickListener { go { openDest(l.dest) } }
                addView(tv(l.label, TypeScale.META, p.muted))
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                    addView(dot(dotColor(l.dot)).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(8) })
                    addView(tv(l.text, 16f, p.text, TypeScale.W_MEDIUM))
                })
                contentDescription = l.description
                for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(2) })
        }
        addView(box, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(12); bottomMargin = dp(12) })
    }

    private fun openDest(d: ShellPages.Dest) = when (d) {
        is ShellPages.Dest.Page -> SettingsActivity.open(this, d.page)
        ShellPages.Dest.OpenRouter -> OpenRouterActivity.open(this)
    }

    private fun dotColor(d: ShellPages.Dot) = when (d) {
        ShellPages.Dot.OK -> ui.t.dotOk
        ShellPages.Dot.CLOUD -> ui.t.dotWarn
        ShellPages.Dot.OFF -> ui.t.ink2
        ShellPages.Dot.BAD -> ui.t.danger
    }

    private fun openPermissions() = startActivity(PermissionsActivity.intent(this))

    /** Speech engines are set in Settings → Speech to text (Developer settings keep only the server). */
    private fun openEngines() = SettingsActivity.open(this, SettingsPages.Page.SPEECH)

    private fun extra(e: Extra) {
        when (e) {
            Extra.NOTES -> startActivity(Intent(this, NotesActivity::class.java))
            Extra.QUICK_NOTE -> startActivity(NoteEditorActivity.scratchpad(this))
            Extra.INSIGHTS -> startActivity(Intent(this, InsightsActivity::class.java))
            Extra.HISTORY -> startActivity(Intent(this, HistoryActivity::class.java))
            Extra.CALLS -> startActivity(Intent(this, CallsActivity::class.java))
            Extra.START_BUBBLE -> startBubble()
            Extra.STOP_BUBBLE -> runCatching { startService(Intent(this, MurmurService::class.java).setAction(MurmurService.ACTION_STOP)) }
            Extra.SPEECH_MODEL -> startActivity(Intent(this, ModelImportActivity::class.java))
            Extra.DEVELOPER -> startActivity(Intent(this, DeveloperActivity::class.java))
        }
    }

    private fun startBubble() {
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "Allow Display over other apps first (Permissions).", Toast.LENGTH_LONG).show()
            return
        }
        runCatching { startForegroundService(Intent(this, MurmurService::class.java).setAction(MurmurService.ACTION_SHOW)) }
    }

    private companion object {
        const val STATE_TAB = "tab"
        val EXTRA_ICONS = mapOf(
            Extra.NOTES to R.drawable.ic_scr_snippet, Extra.QUICK_NOTE to R.drawable.ic_scr_file, Extra.INSIGHTS to R.drawable.ic_scr_chart,
            Extra.HISTORY to R.drawable.ic_set_history, Extra.CALLS to R.drawable.ic_scr_file, Extra.START_BUBBLE to R.drawable.ic_scr_play, Extra.STOP_BUBBLE to R.drawable.ic_scr_stop,
            Extra.SPEECH_MODEL to R.drawable.ic_set_mic, Extra.DEVELOPER to R.drawable.ic_set_code,
        )
    }
}
