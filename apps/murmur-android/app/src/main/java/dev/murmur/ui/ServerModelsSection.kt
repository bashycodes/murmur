package dev.flowclone.ui

import android.app.Activity
import android.widget.LinearLayout
import dev.flowclone.data.ModelPages
import java.io.File

/**
 * D41: the "From your server" block of a model screen ([ModelImportActivity]). Only the hybrid /
 * remote flavours have one (`dev.flowclone.server.ServerModels`, src/server); the local
 * flavour's stand-in (src/noserver) returns none, so it stays import-only with no network code.
 */
internal interface ServerModelsSection {
    /** Adds the block to [parent] (called from the screen's render). */
    fun render(parent: LinearLayout)

    /** The screen goes away; a running download goes on and is shown again when the screen is back. */
    fun detach()
}

/** What the block needs from its screen. */
internal interface ServerModelsHost {
    val activity: Activity
    val ui: DevUi
    val screen: ModelPages.Screen

    /** The file / folder this screen's slot uses now (null = none). */
    fun activePath(): String?

    /** A model finished (or was chosen): point the slot at [target], reload, toast and re-render. */
    fun use(target: File)

    /** Re-render the screen (keeps the scroll position). */
    fun refresh()
}
