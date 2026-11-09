package dev.flowclone.debug

import dev.flowclone.BuildConfig
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File

/**
 * Release builds: the debug "WAV as microphone" receiver is absent (class and manifest entry),
 * so nothing outside the app can make a release build dictate. The build's
 * verify<Variant>NoDebugWavReceiver gate checks the same merged manifest on assemble / check.
 */
class NoDebugWavReceiverTest {
    private val variant = BuildConfig.FLAVOR + BuildConfig.BUILD_TYPE.replaceFirstChar { it.uppercase() }

    @Test fun isAReleaseBuild() = assertFalse(BuildConfig.DEBUG)

    @Test fun receiverClassIsNotInRelease() {
        try {
            Class.forName("dev.flowclone.debug.WavMicReceiver")
            fail("WavMicReceiver is on the $variant classpath")
        } catch (_: ClassNotFoundException) {
        }
    }

    @Test fun benchActivityIsNotInRelease() {
        try {
            Class.forName("dev.flowclone.debug.BenchActivity")
            fail("BenchActivity is on the $variant classpath")
        } catch (_: ClassNotFoundException) {
        }
    }

    @Test fun callDecodeReceiverIsNotInRelease() {
        try {
            Class.forName("dev.flowclone.debug.CallDecodeReceiver")
            fail("CallDecodeReceiver is on the $variant classpath")
        } catch (_: ClassNotFoundException) {
        }
    }

    @Test fun mergedReleaseManifestHasNoWavReceiver() {
        val cap = variant.replaceFirstChar { it.uppercase() }
        val merged = File("build/intermediates/merged_manifest/$variant/process${cap}MainManifest/AndroidManifest.xml")
        assertTrue("merged manifest missing at ${merged.absolutePath}", merged.isFile)
        val xml = merged.readText()
        assertFalse("WavMicReceiver in the $variant manifest", xml.contains("WavMicReceiver"))
        assertFalse(xml.contains(DebugWav.BROADCAST))
        assertFalse("BenchActivity in the $variant manifest", xml.contains("BenchActivity"))
        assertFalse("CallDecodeReceiver in the $variant manifest", xml.contains("CallDecodeReceiver"))
    }
}
