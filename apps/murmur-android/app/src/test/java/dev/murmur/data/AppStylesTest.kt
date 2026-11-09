package dev.murmur.data

import dev.murmur.policy.AppCategorizer.Category
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Style per app as an app list (ui/AppStylesPage): rows, the "Add app" list, edits, and the stored format. */
class AppStylesTest {
    private val gmail = AppStyles.App("com.google.android.gm", "Gmail")
    private val slack = AppStyles.App("com.Slack", "Slack")
    private val notes = AppStyles.App("com.example.notes", "notes")
    private val zoom = AppStyles.App("us.zoom.videomeetings", "Zoom")

    @Test fun rowsLabelInstalledAppsAndKeepUninstalledOnesByPackage() {
        val rows = AppStyles.rows(
            mapOf("com.gone.app" to Category.WORK, "com.Slack" to Category.EMAIL, "com.example.notes" to Category.PERSONAL),
            listOf(slack, notes, gmail),
        )
        assertEquals(listOf("com.gone.app", "notes", "Slack"), rows.map { it.title })
        val gone = rows.first()
        assertFalse(gone.installed)
        assertEquals(Category.WORK, gone.category)
        assertTrue(rows[2].installed)
        assertEquals(Category.EMAIL, rows[2].category)
    }

    @Test fun candidatesPutRecentAppsFirstAndSkipOverridden() {
        val all = listOf(zoom, slack, gmail, notes)
        val recent = listOf("com.Slack", "com.gone.app", "com.Slack", "com.google.android.gm")
        assertEquals(listOf(slack, gmail, notes, zoom), AppStyles.candidates(all, emptyMap(), recent))
        assertEquals(listOf(gmail, notes, zoom), AppStyles.candidates(all, mapOf("com.Slack" to Category.PERSONAL), recent))
        assertEquals(listOf(gmail, notes, slack, zoom), AppStyles.candidates(all, emptyMap(), emptyList()))
    }

    @Test fun candidatesSearchLabelAndPackage() {
        val all = listOf(zoom, slack, gmail, notes)
        assertEquals(listOf(gmail), AppStyles.candidates(all, emptyMap(), emptyList(), " GMA "))
        assertEquals(listOf(zoom), AppStyles.candidates(all, emptyMap(), emptyList(), "videomeet"))
        assertTrue(AppStyles.candidates(all, emptyMap(), emptyList(), "nothing like it").isEmpty())
    }

    @Test fun builtInGroupIsSuggested() {
        assertEquals(Category.EMAIL, AppStyles.suggested("com.google.android.gm"))
        assertEquals("Built-in: Email", AppStyles.builtIn("com.google.android.gm"))
        assertEquals(Category.OTHER, AppStyles.suggested("com.example.notes"))
        assertEquals(listOf("Personal", "Work", "Email", "Other"), AppStyles.GROUPS.map { it.second })
    }

    @Test fun setMovesAndRemoveDeletes() {
        var m = AppStyles.set(emptyMap(), "com.google.android.gm", Category.EMAIL)
        m = AppStyles.set(m, "com.Slack", Category.PERSONAL)
        m = AppStyles.set(m, "com.Slack", Category.WORK)
        assertEquals(mapOf("com.google.android.gm" to Category.EMAIL, "com.Slack" to Category.WORK), m)
        assertEquals(mapOf("com.Slack" to Category.WORK), AppStyles.remove(m, "com.google.android.gm"))
        assertEquals(m, AppStyles.remove(m, "com.not.there"))
    }

    @Test fun pickedOverridesRoundTripThroughTheStoredFormat() {
        // The picker writes the same `app_buckets` text the typed page did, so both directions load.
        val picked = AppStyles.set(AppStyles.set(emptyMap(), "com.google.android.gm", Category.EMAIL), "org.foo_bar.Baz9", Category.OTHER)
        val stored = WritingStyle.formatOverrides(picked)
        assertEquals("com.google.android.gm = email\norg.foo_bar.Baz9 = other", stored)
        assertEquals(picked, WritingStyle.parseOverrides(stored).map)
        val typed = "com.example.chat = Work\n com.x.mail=email "
        assertEquals(
            mapOf("com.example.chat" to Category.WORK, "com.x.mail" to Category.EMAIL),
            WritingStyle.parseOverrides(WritingStyle.formatOverrides(WritingStyle.parseOverrides(typed).map)).map,
        )
    }

    // ---- Style tab: the apps a group's style applies in ----

    @Test fun inGroupListsMovedAppsFirstThenTheBuiltInOnes() {
        val whatsapp = AppStyles.App("com.whatsapp", "WhatsApp")
        val all = listOf(zoom, whatsapp, slack, gmail, notes, whatsapp)
        assertEquals(listOf(whatsapp), AppStyles.inGroup(all, emptyMap(), Category.PERSONAL))
        assertEquals(listOf(gmail), AppStyles.inGroup(all, emptyMap(), Category.EMAIL))
        // Moved apps lead (the user's choice), and leave their built-in group.
        val moved = mapOf("us.zoom.videomeetings" to Category.PERSONAL, "com.google.android.gm" to Category.OTHER)
        assertEquals(listOf(zoom, whatsapp), AppStyles.inGroup(all, moved, Category.PERSONAL))
        assertTrue(AppStyles.inGroup(all, moved, Category.EMAIL).isEmpty())
        assertEquals(listOf(gmail, notes), AppStyles.inGroup(all, moved, Category.OTHER))
        assertEquals(Category.WORK, AppStyles.groupOf("com.Slack", emptyMap()))
        assertEquals(Category.EMAIL, AppStyles.groupOf("com.Slack", mapOf("com.Slack" to Category.EMAIL)))
    }

    @Test fun captionsAndIconRowDescription() {
        assertEquals("Used when you dictate into chat apps for friends and family", AppStyles.caption(Category.PERSONAL))
        assertEquals("Used in every other app", AppStyles.caption(Category.OTHER))
        assertEquals("No apps on this phone in Work yet", AppStyles.appsDescription(Category.WORK, emptyList(), 5))
        assertEquals("Apps in Email: Gmail", AppStyles.appsDescription(Category.EMAIL, listOf(gmail), 5))
        assertEquals("Apps in Other: Zoom and 2 more", AppStyles.appsDescription(Category.OTHER, listOf(zoom, notes, slack), 1))
    }
}
