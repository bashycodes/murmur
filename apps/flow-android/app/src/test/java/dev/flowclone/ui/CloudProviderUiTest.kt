package dev.flowclone.ui

import android.content.Intent
import android.view.accessibility.AccessibilityEvent
import android.widget.EditText
import dev.flowclone.FlowApp
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Cloud provider page (D18) in the Settings look: sections, switch rows, and the API key never in the a11y tree. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class CloudProviderUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()

    @Before fun cloudBuild() = assumeTrue(FlowApp.of(ctx).caps.cloud)

    private fun launch() = Robolectric.buildActivity(
        CloudProviderActivity::class.java,
        Intent(ctx, CloudProviderActivity::class.java).putExtra(CloudProviderActivity.EXTRA_PRESET, "openrouter"),
    ).setup().visible().get()

    @Test fun sectionsAndRowsArePresent() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("PROVIDER", "API KEY", "WHAT IS SENT", "CONSENT", "Model", "Base URL", "Stored key", "Show", "Paste", "Clear stored key", "Add provider", "Test provider", "Send dictionary words", "Zero retention")) {
            assertTrue(want, want in t)
        }
        val sw = rowTitled(a, "Send on-screen words").createAccessibilityNodeInfo()
        assertEquals(android.widget.Switch::class.java.name, sw.className)
        assertFalse("off by default", sw.isChecked)
        UiTestKit.assertTouchTargets(a, "cloud provider")
    }

    @Test fun typedKeyNeverReachesTheA11yTree() {
        val a = launch()
        val key = walk(a.window.decorView).filterIsInstance<EditText>().first { it.createAccessibilityNodeInfo().isPassword }
        key.setText("sk-or-SUPERSECRET")
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Show" }.performClick()
        for (v in walk(a.window.decorView)) {
            val n = v.createAccessibilityNodeInfo()
            val strings = listOf(n.text, n.contentDescription, n.hintText, n.stateDescription, n.error).map { it?.toString() }
            assertFalse(v.javaClass.simpleName, strings.any { it?.contains("SUPERSECRET") == true })
            val ev = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED)
            v.onInitializeAccessibilityEvent(ev); v.dispatchPopulateAccessibilityEvent(ev)
            assertFalse(ev.text.any { it?.contains("SUPERSECRET") == true })
        }
        assertEquals("set (hidden)", key.createAccessibilityNodeInfo().text.toString())
    }
}
