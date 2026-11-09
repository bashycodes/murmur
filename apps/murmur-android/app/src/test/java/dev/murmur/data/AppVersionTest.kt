package dev.flowclone.data

import dev.flowclone.BuildConfig
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File

/**
 * D26 / docs/versioning.md: the product version rules. The vectors are
 * `tools/versioning/fixtures/version_vectors.json`, which `tools/versioning/test_versioning.py`
 * checks against the Python rules used by `tools/check-version.sh` and `tools/release.sh`.
 */
class AppVersionTest {
    private val f: JSONObject = JSONObject(
        requireNotNull(javaClass.classLoader!!.getResource("version_vectors.json")) { "fixture not on the test classpath" }.readText(),
    )

    @Test fun validVersionsGiveTheirVersionCode() {
        val a = f.getJSONArray("valid")
        assertTrue(a.length() >= 5)
        for (i in 0 until a.length()) {
            val c = a.getJSONObject(i)
            assertEquals(c.getString("version"), c.getInt("code"), AppVersion.code(c.getString("version")))
        }
    }

    @Test fun invalidVersionsAreRejected() {
        val a = f.getJSONArray("invalid")
        for (i in 0 until a.length()) {
            val v = a.getString(i)
            try {
                AppVersion.parse(v)
                fail("accepted ${JSONObject.quote(v)}")
            } catch (_: IllegalArgumentException) {
            }
        }
    }

    @Test fun describeGivesBuildAndFull() {
        val a = f.getJSONArray("describe")
        for (i in 0 until a.length()) {
            val c = a.getJSONObject(i)
            val (build, full) = AppVersion.describe(c.getString("version"), c.getString("sha"), c.getBoolean("dirty"))
            assertEquals(c.toString(), c.getString("build"), build)
            assertEquals(c.toString(), c.getString("full"), full)
        }
    }

    /** build.gradle.kts duplicates the formula (it cannot see app code); this pins the two together. */
    @Test fun buildConfigIsDerivedFromTheVersionFile() {
        val file = File(requireNotNull(System.getProperty("flow.versionFile")) { "flow.versionFile not set by build.gradle.kts" })
        assertEquals(file.readText().trim(), BuildConfig.VERSION_NAME)
        assertEquals(AppVersion.code(BuildConfig.VERSION_NAME), BuildConfig.VERSION_CODE)
        assertEquals(AppVersion.describe(BuildConfig.VERSION_NAME, BuildConfig.GIT_SHA, BuildConfig.GIT_DIRTY).first, BuildConfig.BUILD)
        assertEquals(BuildConfig.VERSION_NAME, AppVersion.current.version)
        assertEquals(BuildConfig.BUILD, AppVersion.current.build)
    }

    @Test fun labelsNameVersionFlavourAndBuild() {
        val b = AppBuild(version = "0.2.0", code = 200, build = "abc1234-dirty", full = "0.2.0+abc1234.dirty")
        assertEquals("Version 0.2.0 (hybrid) · abc1234-dirty", b.label("hybrid"))
        assertEquals("0.2.0 (200) · build abc1234-dirty", b.detail())
        assertFalse(b.label("local").contains("null"))
    }
}
