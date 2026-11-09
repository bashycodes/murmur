package dev.flowclone.ui.tabs

import android.app.Activity
import android.app.AlertDialog
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind
import dev.flowclone.data.ShellPages
import dev.flowclone.ui.DevUi
import dev.flowclone.ui.DictionaryActivity
import dev.flowclone.ui.ShellTab

/**
 * Shared body of the Dictionary and Snippets tabs (AndroidDictionary / AndroidSnippets.dc.html):
 * an outlined search box pinned above a scrolling list of [kind]'s entries, and the "+" FAB at the
 * bottom right (above the bottom nav) that opens the add page. Tapping an entry opens its edit page
 * ([DictionaryActivity]); long-press / the TalkBack "Delete" action deletes it (asks first). The
 * top bar's "Import or export" action keeps the shared JSON file reachable. Reloads on [onShow]
 * (the edit page and import return here).
 */
internal abstract class EntryTab(protected val activity: Activity, protected val ui: DevUi, val kind: Kind) : ShellTab {
    protected val app get() = FlowApp.of(activity)
    override val title = ShellPages.APP_NAME

    private var view: View? = null
    /** The scrolling column the rows go in (tests read it). */
    lateinit var content: LinearLayout; private set
    lateinit var search: EditText; private set
    lateinit var fab: View; private set
    private var query = ""

    override fun actions() = listOf(DevUi.TopAction(R.drawable.ic_scr_share, "Import or export") { transfer() })

    override fun createView(): View = view ?: build().also { view = it }

    override fun onShow() = fill()

    private fun build(): View = with(ui) {
        val col = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
        val searchBox = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(8), dp(16), 0)
        }
        search = searchBox.searchField("Search", query) { q -> query = q; fill() }.apply {
            contentDescription = DictionaryPages.searchLabel(kind)
        }
        col.addView(searchBox)
        val (scroll, inner) = scrollColumn(null, topDp = 4) {}
        // Room under the last row so the FAB never covers it.
        inner.setPadding(inner.paddingLeft, inner.paddingTop, inner.paddingRight, dp(96))
        content = inner
        col.addView(scroll, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        val root = FrameLayout(activity)
        root.addView(col, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        fab = fab(R.drawable.ic_lw_plus, DictionaryPages.addLabel(kind)) { DictionaryActivity.edit(activity, kind, null) }
        root.addView(fab)
        root
    }

    /** Re-renders the list from the store (cheap: a small in-memory list). */
    fun fill(): Unit = with(ui) {
        if (view == null) createView()
        content.removeAllViews()
        above(content)
        val all = app.dictionary.all()
        val shown = DictionaryPages.entries(all, kind, query)
        if (shown.isEmpty()) {
            val e = DictionaryPages.empty(kind, query, DictionaryPages.of(all, kind).size)
            content.card { emptyState(e.title, e.hint) }
            return@with
        }
        rows(content, shown)
    }

    /** Anything shown above the list (Dictionary: learn-from-edits suggestions). */
    protected open fun above(col: LinearLayout) {}

    /** The entries, already filtered and sorted. */
    protected abstract fun rows(col: LinearLayout, entries: List<DictEntry>)

    /** Tap = edit; long-press and the TalkBack "Delete" action delete (after asking). */
    protected fun View.entry(e: DictEntry) {
        isClickable = true; isFocusable = true
        minimumHeight = ui.dp(58)
        contentDescription = DictionaryPages.row(e).description
        setOnClickListener { DictionaryActivity.edit(activity, kind, e.word) }
        setOnLongClickListener { confirmDelete(e); true }
        val deleteId = View.generateViewId()
        accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                super.onInitializeAccessibilityNodeInfo(host, info)
                info.addAction(AccessibilityNodeInfo.AccessibilityAction(deleteId, "Delete"))
            }
            override fun performAccessibilityAction(host: View, action: Int, args: android.os.Bundle?): Boolean {
                if (action == deleteId) { confirmDelete(e); return true }
                return super.performAccessibilityAction(host, action, args)
            }
        }
        if (this is ViewGroup) for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
    }

    private fun confirmDelete(e: DictEntry) {
        ui.confirm("Delete “${e.word}”?", if (e.isSnippet) "The snippet and its text are removed." else "The word is removed from the dictionary.", "Delete", onYes = {
            app.dictionary.remove(e.word)
            ui.announce("Deleted ${e.word}")
            fill()
        })
    }

    /** Import / export the shared `[{word, replacement, isSnippet}]` file (the Windows app reads the same one). */
    private fun transfer() {
        AlertDialog.Builder(activity)
            .setTitle("Dictionary file")
            .setItems(arrayOf("Import JSON", "Export JSON")) { _, which -> DictionaryActivity.transfer(activity, import = which == 0) }
            .setNegativeButton("Cancel", null)
            .show()
    }
}
