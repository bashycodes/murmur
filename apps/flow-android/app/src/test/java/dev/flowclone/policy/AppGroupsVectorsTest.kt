package dev.flowclone.policy

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The shared app-group vectors (crates/flow-core/fixtures/app-groups/vectors.json, on the test
 * classpath via sourceSets). flow-core's tests/app_groups_vectors.rs checks the same file, so
 * PolicyLists and the Rust core read crates/flow-core/data/app-groups.json the same way (D80).
 */
class AppGroupsVectorsTest {
    private val vectors = JSONObject(javaClass.classLoader!!.getResource("app-groups/vectors.json")!!.readText())
        .getJSONArray("vectors")

    /** The predicate each Android caller uses, by group name. */
    private val predicates: Map<String, (String?) -> Boolean> = mapOf(
        "restricted" to BubbleVisibilityPolicy::isRestricted,
        "restart_prone" to PolicyLists::isRestartProne,
        "known_null" to PolicyLists::isKnownNullSurroundingText,
        "browser" to PolicyLists::isBrowser,
        "system_ui" to PolicyLists::isSystemUi,
        "system_search" to PolicyLists::isSystemSearch,
        "in_call" to PolicyLists::isInCall,
        "compose_placeholder_apps" to PolicyLists::usesComposePlaceholders,
    )

    private fun bucket(c: AppCategorizer.Category) = when (c) {
        AppCategorizer.Category.EMAIL -> "email"
        AppCategorizer.Category.WORK -> "work"
        AppCategorizer.Category.PERSONAL -> "personal"
        AppCategorizer.Category.OTHER -> "other"
    }

    @Test fun androidVectorsMatch() {
        var checked = 0
        val fails = mutableListOf<String>()
        for (i in 0 until vectors.length()) {
            val v = vectors.getJSONObject(i)
            if (v.getString("platform") != "android") continue
            val app = v.optString("app").ifEmpty { null }
            val host = v.optString("host").ifEmpty { null }
            val (want, got) = if (v.has("bucket")) {
                v.getString("bucket") to bucket(AppCategorizer.categorize(app, host))
            } else {
                val p = predicates[v.getString("group")] ?: error("no predicate for ${v.getString("group")}")
                v.getBoolean("expect").toString() to p(app).toString()
            }
            if (want != got) fails += "$v: got $got"
            checked++
        }
        assertTrue(fails.joinToString("\n"), fails.isEmpty())
        assertTrue(checked > 50)
    }

    @Test fun placeholdersAreNormalized() {
        assertTrue(PolicyLists.COMPOSE_PLACEHOLDERS.isNotEmpty())
        for (s in PolicyLists.COMPOSE_PLACEHOLDERS)
            assertEquals(s, dev.flowclone.insert.TextMatching.normalizePlaceholder(s))
    }

    @Test fun nullAndBlankMatchNothing() {
        for (p in predicates.values) {
            assertEquals(false, p(null))
            assertEquals(false, p(" "))
        }
    }

    @Test fun packagedGroupsLoad() {
        assertTrue(PolicyLists.loaded)
    }

    /** A missing or broken file must not crash the service and must hide the bubble everywhere. */
    @Test fun brokenGroupsTreatEveryAppAsRestricted() {
        for (text in listOf(null, "", "{", """{"android": {}}""", """{"android": {"restricted": {}}}""")) {
            val d = PolicyLists.load(text)
            assertEquals(text, false, d.ok)
            for (app in listOf("com.example.notes", "com.whatsapp", "com.android.chrome"))
                assertTrue("$text / $app", PolicyLists.isRestricted(d, app))
            assertEquals(text, false, PolicyLists.group(d, "browser").app("com.android.chrome"))
        }
    }
}
