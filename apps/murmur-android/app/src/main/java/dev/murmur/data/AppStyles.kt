package dev.murmur.data

import dev.murmur.policy.AppCategorizer
import dev.murmur.policy.AppCategorizer.Category

/**
 * Settings → Writing style → Style per app: the per-app bucket overrides ([Settings.appBuckets])
 * as a list of apps instead of typed `package = bucket` lines. Storage is unchanged (the
 * `app_buckets` pref in [WritingStyle.formatOverrides] form), so overrides typed before this page
 * load as rows. Pure logic; the page (ui/AppStylesPage) supplies the installed apps.
 */
object AppStyles {
    /** A launchable app on this phone (launcher-intent query; the manifest declares that `<queries>`). */
    data class App(val pkg: String, val label: String)

    /** One override row. [label] is null when the package is not on this phone (uninstalled): the row shows the package. */
    data class Row(val pkg: String, val label: String?, val category: Category) {
        val title: String get() = label ?: pkg
        val installed: Boolean get() = label != null
    }

    /** The group choices, in page order (same titles as the Writing style sections, short form). */
    val GROUPS: List<Pair<Category, String>> = listOf(
        Category.PERSONAL to "Personal", Category.WORK to "Work", Category.EMAIL to "Email", Category.OTHER to "Other",
    )

    fun groupName(c: Category): String = GROUPS.first { it.first == c }.second

    /** Rows for [overrides], by title (case-insensitive), then package; [installed] labels the known ones. */
    fun rows(overrides: Map<String, Category>, installed: List<App>): List<Row> {
        val labels = installed.associate { it.pkg to it.label }
        return overrides.map { (pkg, c) -> Row(pkg, labels[pkg], c) }
            .sortedWith(compareBy<Row, String>(String.CASE_INSENSITIVE_ORDER) { it.title }.thenBy { it.pkg })
    }

    /** "Built-in: Email" meta line: the group this app would get without an override. */
    fun builtIn(pkg: String): String = "Built-in: " + groupName(AppCategorizer.categorize(pkg, null))

    /**
     * The "Add app" list: launchable apps without an override yet, matching [query] (label or
     * package, case-insensitive). Apps dictated into recently ([recent], newest first, may repeat
     * or name apps that are gone) come first in that order, then the rest by label.
     */
    fun candidates(installed: List<App>, overrides: Map<String, Category>, recent: List<String>, query: String = ""): List<App> {
        val q = query.trim()
        val open = installed.filter { it.pkg !in overrides }
            .filter { q.isEmpty() || it.label.contains(q, ignoreCase = true) || it.pkg.contains(q, ignoreCase = true) }
            .distinctBy { it.pkg }
        val rank = HashMap<String, Int>()
        for (p in recent) if (p !in rank) rank[p] = rank.size
        val (front, rest) = open.partition { it.pkg in rank }
        return front.sortedBy { rank.getValue(it.pkg) } +
            rest.sortedWith(compareBy<App, String>(String.CASE_INSENSITIVE_ORDER) { it.label }.thenBy { it.pkg })
    }

    /** The group the picker preselects for [pkg]: its built-in one. */
    fun suggested(pkg: String): Category = AppCategorizer.categorize(pkg, null)

    /** [overrides] with [pkg] put in [c] (added or moved). */
    fun set(overrides: Map<String, Category>, pkg: String, c: Category): Map<String, Category> = LinkedHashMap(overrides).apply { put(pkg, c) }

    fun remove(overrides: Map<String, Category>, pkg: String): Map<String, Category> = overrides - pkg

    // ---- Style tab (AndroidStyle.dc.html): the apps a group's style applies in ----

    /** The group dictation into [pkg] gets: the user's override, else the built-in list ([AppCategorizer]). */
    fun groupOf(pkg: String, overrides: Map<String, Category>): Category = overrides[pkg] ?: AppCategorizer.categorize(pkg, null)

    /**
     * The launchable apps whose dictation gets [c]'s style: apps the user moved into [c] first
     * (their own choice), then the built-in list's; each part by label (case-insensitive), then package.
     */
    fun inGroup(installed: List<App>, overrides: Map<String, Category>, c: Category): List<App> {
        val byLabel = compareBy<App, String>(String.CASE_INSENSITIVE_ORDER) { it.label }.thenBy { it.pkg }
        val (moved, builtIn) = installed.distinctBy { it.pkg }.filter { groupOf(it.pkg, overrides) == c }.partition { it.pkg in overrides }
        return moved.sortedWith(byLabel) + builtIn.sortedWith(byLabel)
    }

    /** The line under the app icons on the Style tab. */
    fun caption(c: Category): String = when (c) {
        Category.PERSONAL -> "Used when you dictate into chat apps for friends and family"
        Category.WORK -> "Used when you dictate into team chat apps"
        Category.EMAIL -> "Used when you dictate into email"
        Category.OTHER -> "Used in every other app"
    }

    /** TalkBack text of the icon row: "Apps in Personal: WhatsApp, Signal and 3 more" ([shown] icons of [apps]). */
    fun appsDescription(c: Category, apps: List<App>, shown: Int): String = when {
        apps.isEmpty() -> "No apps on this phone in ${groupName(c)} yet"
        apps.size <= shown -> "Apps in ${groupName(c)}: " + apps.joinToString(", ") { it.label }
        else -> "Apps in ${groupName(c)}: " + apps.take(shown).joinToString(", ") { it.label } + " and ${apps.size - shown} more"
    }
}
