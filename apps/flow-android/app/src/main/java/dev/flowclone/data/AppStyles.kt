package dev.flowclone.data

import dev.flowclone.policy.AppCategorizer
import dev.flowclone.policy.AppCategorizer.Category

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
}
