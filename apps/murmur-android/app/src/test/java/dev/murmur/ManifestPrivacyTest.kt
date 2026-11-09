package dev.flowclone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Unit-suite twin of the per-flavour manifest gates (D17 §1.4): `local` declares no network
 * permission at all; `hybrid`/`remote` declare INTERNET and no other network permission.
 * The flavour source manifests must actively strip the rest in case a dependency adds them.
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

    private val flavor = BuildConfig.FLAVOR
    private val variant = flavor + BuildConfig.BUILD_TYPE.replaceFirstChar { it.uppercase() }

    @Test fun mergedManifestHasOnlyTheFlavoursNetworkPermission() {
        val cap = variant.replaceFirstChar { it.uppercase() }
        val merged = File("build/intermediates/merged_manifest/$variant/process${cap}MainManifest/AndroidManifest.xml")
        assertTrue("merged manifest missing at ${merged.absolutePath} (AGP layout changed?)", merged.isFile)
        val perms = permissions(merged.readText())
        val allowed = if (BuildConfig.CAP_REMOTE) setOf("android.permission.INTERNET") else emptySet()
        for (p in forbidden - allowed) assertFalse("$p in merged $variant manifest", p in perms)
        for (p in allowed) assertTrue("$p expected in $variant", p in perms)
        assertTrue("RECORD_AUDIO expected", "android.permission.RECORD_AUDIO" in perms)
    }

    @Test fun flavourCapabilitiesMatchTheFlavourName() {
        assertEquals(flavor != "remote", BuildConfig.CAP_LOCAL)
        assertEquals(flavor != "local", BuildConfig.CAP_REMOTE)
        // Review P4: cloud STT exists exactly in the networked flavours (D18 C2).
        assertEquals(flavor != "local", BuildConfig.CAP_CLOUD)
    }

    /** Review P4: no backup / device-transfer copy of settings, history or sealed keys. */
    @Test fun backupAndDataExtractionAreOff() {
        val main = File("src/main/AndroidManifest.xml").readText()
        assertTrue(Regex("""android:allowBackup="false"""").containsMatchIn(main))
        val rules = Regex("""android:dataExtractionRules="@xml/([a-z_]+)"""").find(main)?.groupValues?.get(1)
        assertTrue("dataExtractionRules missing", rules != null)
        val xml = File("src/main/res/xml/$rules.xml").readText()
        for (section in listOf("cloud-backup", "device-transfer")) assertTrue("$section in $rules.xml", xml.contains("<$section"))
        for (domain in listOf("root", "file", "database", "sharedpref")) {
            assertEquals("$domain excluded in both sections", 2, Regex("""<exclude\s+domain="$domain"\s+path="\."""").findAll(xml).count())
        }
    }

    @Test fun flavourManifestsStripOrAddExactlyTheRightPermissions() {
        val local = File("src/local/AndroidManifest.xml").readText()
        assertTrue(Regex("""android\.permission\.INTERNET"\s+tools:node="remove"""").containsMatchIn(local))
        for (f in listOf("hybrid", "remote")) {
            val m = File("src/$f/AndroidManifest.xml").readText()
            assertTrue(f, Regex("""android\.permission\.INTERNET"\s*/>""").containsMatchIn(m))
            assertTrue(f, Regex("""ACCESS_NETWORK_STATE"\s+tools:node="remove"""").containsMatchIn(m))
        }
        val main = File("src/main/AndroidManifest.xml").readText()
        assertFalse("main manifest must not decide network permissions", main.contains("android.permission.INTERNET"))
        assertFalse(main.contains("usesCleartextTraffic=\"true\""))
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
