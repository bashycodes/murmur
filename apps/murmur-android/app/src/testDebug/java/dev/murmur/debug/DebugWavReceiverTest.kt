package dev.flowclone.debug

import dev.flowclone.BuildConfig
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** Debug builds: the "WAV as microphone" receiver is there, and only the shell can trigger it. */
class DebugWavReceiverTest {
    private val variant = BuildConfig.FLAVOR + BuildConfig.BUILD_TYPE.replaceFirstChar { it.uppercase() }

    @Test fun receiverClassShipsInDebug() {
        Class.forName("dev.flowclone.debug.WavMicReceiver")
    }

    @Test fun mergedDebugManifestDeclaresItPermissionGuarded() {
        val cap = variant.replaceFirstChar { it.uppercase() }
        val merged = File("build/intermediates/merged_manifest/$variant/process${cap}MainManifest/AndroidManifest.xml")
        assertTrue("merged manifest missing at ${merged.absolutePath}", merged.isFile)
        val receiver = Regex("""<receiver\b[^>]*WavMicReceiver[^>]*>""").find(merged.readText())?.value
        assertTrue("WavMicReceiver missing from the $variant manifest", receiver != null)
        assertTrue(receiver, receiver!!.contains("""android:permission="android.permission.DUMP""""))
        assertTrue(receiver, receiver.contains("""android:exported="true""""))
    }
}
