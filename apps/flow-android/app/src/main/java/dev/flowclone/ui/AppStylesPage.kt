package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.data.AppStyles
import dev.flowclone.data.Settings
import dev.flowclone.policy.AppCategorizer.Category
import kotlin.concurrent.thread

/**
 * Settings → Writing style → Style per app ([dev.flowclone.data.SettingsPages.Page.APP_STYLES]):
 * one row per override (app icon + label, group dropdown, remove) and "Add app", a searchable list
 * of launchable apps with the ones dictated into recently first. Changes apply at once through
 * [update] (SettingsActivity.applyNow); storage is [Settings.appBuckets] as before.
 *
 * The Style tab ([dev.flowclone.ui.tabs.StyleTab]) uses it without [render]: [apps] / [icon] for
 * its icon row ([onLoaded] when they arrive) and [openPicker] for its "+".
 *
 * App visibility: the manifest's `<queries>` launcher intent (not QUERY_ALL_PACKAGES), so an
 * override for an app without a launcher icon reads like an uninstalled one (package name only).
 */
internal class AppStylesPage(
    private val activity: Activity,
    private val ui: DevUi,
    private val update: (Settings.() -> Settings) -> Unit,
) {
    private val app get() = FlowApp.of(activity)
    private val pm: PackageManager get() = activity.packageManager
    private lateinit var list: LinearLayout

    /** Launchable apps + icons and recent target apps, loaded off the main thread; null until then. */
    private var installed: List<AppStyles.App>? = null
    private val icons = HashMap<String, Drawable?>()
    private var recent: List<String> = emptyList()
    private var picker: (() -> Unit)? = null
    private var loading = false

    /** Called on the main thread once the launchable apps (and icons) are loaded. */
    var onLoaded: (() -> Unit)? = null

    /** Launchable apps, null until loaded ([load]). */
    val apps: List<AppStyles.App>? get() = installed
    /** The overrides the rows show now: a refresh rebuilds only when the stored map differs (a dropdown change does not). */
    private var shown: Map<String, Category>? = null

    private fun dp(v: Int) = (v * activity.resources.displayMetrics.density).toInt()

    fun render() = with(ui) {
        screen {
            note("Put an app in another group than the built-in list does (for example a work chat app the list calls personal).")
            list = card { }
            action("Add app") { pick() }
            note("Changes apply at once.")
        }
        fill()
        load()
    }

    /** Called from SettingsActivity.onResume via the page's refresher: another screen may have changed the overrides. */
    fun refresh() { if (::list.isInitialized) fill() }

    /** Loads the launchable apps and recent target apps off the main thread ([force]: again, e.g. apps were installed). */
    fun load(force: Boolean = false) {
        if (loading || (installed != null && !force)) return
        loading = true
        thread(name = "app-styles-load") {
            val apps = runCatching { launchable() }.getOrDefault(emptyList())
            // Cheap: the last 500 history rows, newest first; empty when history is off (Never save).
            val r = runCatching { app.history.recent().mapNotNull { it.appId?.takeIf(String::isNotBlank) } }.getOrDefault(emptyList())
            activity.runOnUiThread {
                loading = false
                if (activity.isFinishing || activity.isDestroyed) return@runOnUiThread
                recent = r
                installed = apps.map { it.first }
                for ((a, icon) in apps) icons[a.pkg] = icon
                fill(force = true)
                picker?.invoke()
                onLoaded?.invoke()
            }
        }
    }

    private fun launchable(): List<Pair<AppStyles.App, Drawable?>> {
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        return pm.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
            .mapNotNull { ri ->
                val pkg = ri.activityInfo?.packageName ?: return@mapNotNull null
                if (pkg == activity.packageName) return@mapNotNull null
                AppStyles.App(pkg, ri.loadLabel(pm).toString().ifBlank { pkg }) to runCatching { ri.loadIcon(pm) }.getOrNull()
            }
            .distinctBy { it.first.pkg }
    }

    /** Label of an override's app, looked up directly so rows don't wait for the launcher list; null = not on this phone. */
    private fun labelOf(pkg: String): String? = installed?.firstOrNull { it.pkg == pkg }?.label ?: runCatching {
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, PackageManager.ApplicationInfoFlags.of(0))).toString()
    }.getOrNull()

    fun icon(pkg: String): Drawable? = iconOf(pkg)

    private fun iconOf(pkg: String): Drawable? = icons.getOrPut(pkg) { runCatching { pm.getApplicationIcon(pkg) }.getOrNull() }

    private fun fill(force: Boolean = false) = with(ui) {
        if (!::list.isInitialized) return@with
        val overrides = app.settings.load().appBuckets
        if (!force && overrides == shown) return@with
        shown = overrides
        list.removeAllViews()
        val known = overrides.keys.mapNotNull { pkg -> labelOf(pkg)?.let { AppStyles.App(pkg, it) } }
        val rows = AppStyles.rows(overrides, known)
        if (rows.isEmpty()) list.emptyState("No apps moved", "Every app uses the built-in list. Add one below.")
        for (r in rows) list.addView(overrideRow(r))
    }

    private fun appIcon(pkg: String, installed: Boolean): ImageView = ImageView(activity).apply {
        val d = if (installed) iconOf(pkg) else null
        if (d != null) setImageDrawable(d) else {
            setImageResource(R.drawable.ic_set_style); imageTintList = android.content.res.ColorStateList.valueOf(ui.p.muted)
        }
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        layoutParams = LinearLayout.LayoutParams(dp(36), dp(36)).apply { marginEnd = dp(14) }
    }

    private fun texts(title: String, meta: String) = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        addView(TextView(activity).apply { text = title; textSize = 16f; setTextColor(ui.p.text); maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END })
        addView(TextView(activity).apply { text = meta; textSize = 13f; setTextColor(ui.p.muted); maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END })
    }

    private fun overrideRow(r: AppStyles.Row): View {
        val meta = if (r.installed) r.pkg else "Not installed · ${AppStyles.builtIn(r.pkg)}"
        return LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(64)
            setPadding(dp(16), dp(8), dp(4), dp(8))
            addView(appIcon(r.pkg, r.installed))
            addView(texts(r.title, meta), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(groupSpinner(r))
            addView(ui.iconButton(R.drawable.ic_set_delete, "Remove ${r.title}") {
                update { copy(appBuckets = AppStyles.remove(appBuckets, r.pkg)) }
                fill()
                ui.announce("${r.title} removed")
            }.apply { imageTintList = android.content.res.ColorStateList.valueOf(ui.p.danger) })
        }
    }

    private fun groupSpinner(r: AppStyles.Row): Spinner = Spinner(activity, Spinner.MODE_DROPDOWN).apply {
        val names = AppStyles.GROUPS.map { it.second }
        adapter = object : ArrayAdapter<String>(activity, android.R.layout.simple_spinner_item, names) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View =
                (super.getView(position, convertView, parent) as TextView).apply { setTextColor(ui.p.accent); textSize = 15f }
        }.apply { setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item) }
        setSelection(AppStyles.GROUPS.indexOfFirst { it.first == r.category }, false)
        contentDescription = "Group for ${r.title}"
        minimumHeight = dp(48)
        onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                val c = AppStyles.GROUPS[position].first
                val now = app.settings.load().appBuckets
                // The initial selection also lands here: nothing to change then.
                if (now[r.pkg] == c || r.pkg !in now) return
                shown = AppStyles.set(now, r.pkg, c)
                update { copy(appBuckets = AppStyles.set(appBuckets, r.pkg, c)) }
            }
            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
    }

    // ---- Add app ----

    /** "Add app" from elsewhere (Style tab "+"): the same picker; the group step preselects [preselect] (null: the built-in group). */
    fun openPicker(preselect: Category? = null) { load(); pick(preselect) }

    private fun pick(preselect: Category? = null) = with(ui) {
        var query = ""
        lateinit var results: LinearLayout
        lateinit var dialog: AlertDialog
        fun show() {
            results.removeAllViews()
            val all = installed
            if (all == null) { results.note("Loading apps…"); return }
            val overrides = app.settings.load().appBuckets
            val c = AppStyles.candidates(all, overrides, recent, query)
            if (c.isEmpty()) { results.note(if (query.isBlank()) "No other apps to add." else "No app matches “$query”."); return }
            val recentSet = recent.toSet()
            var header: Boolean? = null
            for (a in c) {
                val isRecent = query.isBlank() && a.pkg in recentSet
                if (query.isBlank() && header != isRecent) {
                    results.sectionLabel(if (isRecent) "RECENTLY DICTATED INTO" else "ALL APPS")
                    header = isRecent
                }
                results.addView(pickRow(a) { dialog.dismiss(); chooseGroup(a, preselect) })
            }
        }
        val body = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(8), dp(16), 0)
            searchField("Search apps") { q -> query = q; show() }
            results = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
            addView(ScrollView(activity).apply { addView(results) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(420)))
        }
        dialog = AlertDialog.Builder(activity)
            .setTitle("Add app")
            .setView(body)
            .setNegativeButton("Cancel", null)
            .setOnDismissListener { picker = null }
            .show()
        picker = { show() }
        show()
    }

    private fun pickRow(a: AppStyles.App, onClick: () -> Unit): View = LinearLayout(activity).apply {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
        minimumHeight = dp(56)
        setPadding(dp(4), dp(6), dp(4), dp(6))
        isClickable = true; isFocusable = true
        background = activity.getDrawable(android.R.drawable.list_selector_background)
        setOnClickListener { onClick() }
        addView(appIcon(a.pkg, true))
        addView(texts(a.label, AppStyles.builtIn(a.pkg)), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        contentDescription = "${a.label}, ${AppStyles.builtIn(a.pkg)}"
    }

    /** Second step: which group [a] goes in (preselected: [preselect], else its built-in one). */
    private fun chooseGroup(a: AppStyles.App, preselect: Category?) {
        val names = AppStyles.GROUPS.map { it.second }.toTypedArray()
        val pre = AppStyles.GROUPS.indexOfFirst { it.first == (preselect ?: AppStyles.suggested(a.pkg)) }
        AlertDialog.Builder(activity)
            .setTitle("${a.label}: group")
            .setSingleChoiceItems(names, pre) { d, which ->
                d.dismiss()
                add(a, AppStyles.GROUPS[which].first)
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun add(a: AppStyles.App, c: Category) {
        update { copy(appBuckets = AppStyles.set(appBuckets, a.pkg, c)) }
        fill()
        ui.announce("${a.label} moved to ${AppStyles.groupName(c)}")
    }
}
