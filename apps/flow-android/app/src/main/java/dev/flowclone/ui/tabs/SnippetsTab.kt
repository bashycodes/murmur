package dev.flowclone.ui.tabs

import android.app.Activity
import android.text.TextUtils
import android.view.ViewGroup
import android.widget.LinearLayout
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind
import dev.flowclone.ui.DevUi
import dev.flowclone.ui.TypeScale

/** Snippets tab (AndroidSnippets.dc.html): one card per snippet, what you say over the text it inserts (one line). */
internal class SnippetsTab(activity: Activity, ui: DevUi) : EntryTab(activity, ui, Kind.SNIPPETS) {
    override fun rows(col: LinearLayout, entries: List<DictEntry>): Unit = with(ui) {
        for (e in entries) {
            val r = DictionaryPages.row(e)
            val card = LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
                background = ripple(rounded(p.surface, 24), 24)
                setPadding(dp(20), dp(22), dp(20), dp(22))
                addView(tv(r.word, TypeScale.ROW, p.text).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END })
                addView(tv(r.detail, TypeScale.ROW, t.ink2).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(2) })
                entry(e)
            }
            col.addView(card, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6); bottomMargin = dp(6) })
        }
    }
}
