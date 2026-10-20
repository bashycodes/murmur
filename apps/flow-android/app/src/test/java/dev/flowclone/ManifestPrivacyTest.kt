package dev.flowclone

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Unit-suite twin of the `verify<Variant>NoNetworkPermissions` Gradle gate: the merged
 * manifest (all dependencies included) must declare no network permission, and the
 * source manifest must actively strip INTERNET in case a future dependency adds it.
 * Gradle runs unit tests with the module dir as working directory.
 */
class ManifestPrivacyTest {
    private val forbidden = listOf(
        "android.permission.INTERNET",
        "android.permission.ACCESS_NETWORK_STATE",
        "android.permission.CHANGE_NETWORK_STATE",
        "android.permission.ACCESS_WIFI_STATE",
        "android.permission.CHANGE_WIFI_STATE",
    )

    private fun permissions(xml: String) =
        Regex("""<uses-permission[^>]*android:name="([^"]+)"[^>]*>""").findAll(xml)
            .filterNot { it.value.contains("tools:node=\"remove\"") }
            .map { it.groupValues[1] }.toSet()

    @Test fun mergedManifestHasNoNetworkPermission() {
        val merged = File("build/intermediates/merged_manifest/debug/processDebugMainManifest/AndroidManifest.xml")
        assertTrue("merged manifest missing at ${merged.absolutePath} (AGP layout changed?)", merged.isFile)
        val perms = permissions(merged.readText())
        for (p in forbidden) assertFalse("$p in merged manifest", p in perms)
        assertTrue("RECORD_AUDIO expected", "android.permission.RECORD_AUDIO" in perms)
    }

    @Test fun sourceManifestStripsInternetFromDependencies() {
        val src = File("src/main/AndroidManifest.xml").readText()
        assertTrue(Regex("""android\.permission\.INTERNET"\s+tools:node="remove"""").containsMatchIn(src))
        assertFalse(src.contains("usesCleartextTraffic=\"true\""))
    }

    @Test fun accessibilityConfigHasInputMethodAndNoKeyFilteringOrGestures() {
        val cfg = File("src/main/res/xml/accessibility_service_config.xml").readText()
        assertTrue(cfg.contains("flagInputMethodEditor"))
        assertTrue(cfg.contains("flagRetrieveInteractiveWindows"))
        assertTrue(cfg.contains("typeWindowsChanged"))
        assertFalse(cfg.contains("canPerformGestures"))
        assertFalse(cfg.contains("flagRequestFilterKeyEvents"))
    }
}
