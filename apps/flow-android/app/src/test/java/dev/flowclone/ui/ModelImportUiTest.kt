package dev.flowclone.ui

import android.content.ClipboardManager
import android.content.Intent
import dev.flowclone.BuildConfig
import dev.flowclone.FlowApp
import dev.flowclone.data.ModelFiles
import dev.flowclone.data.ModelPages
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.io.File

/** Model import screens (speech / Parakeet / AI cleanup): status card, Import, recommendations, imported list. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class ModelImportUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    // The screens exist only where there is an on-device engine (not `remote`).
    @Before fun onDevice() = assumeTrue(BuildConfig.CAP_LOCAL)

    @After fun clean() {
        app.engine.modelsDir().listFiles()?.forEach { it.deleteRecursively() }
        app.engine.llmDir().listFiles()?.forEach { it.deleteRecursively() }
        app.settings.modelPath = null; app.settings.llmModelPath = null
    }

    private fun launch(i: Intent = Intent(ctx, ModelImportActivity::class.java)) = Robolectric.buildActivity(ModelImportActivity::class.java, i).setup().visible().get()

    @Test fun speechScreenShowsStatusImportRecommendationsAndEmptyImported() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("STATUS", "Active model", "None imported", "Engine", "whisper.cpp (ggml)", "Import model file…", "RECOMMENDED", "IMPORTED", "None imported yet")) assertTrue(want, want in t)
        for (m in ModelPages.recommended(ModelPages.Screen.WHISPER)) {
            assertTrue(m.file in t); assertTrue("link shown as text", m.url in t)
        }
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Import model file…" }.performClick()
        assertEquals(Intent.ACTION_OPEN_DOCUMENT, shadowOf(a).nextStartedActivityForResult.intent.action)
        UiTestKit.assertTouchTargets(a, "speech")
    }

    @Test fun recommendedRowCopiesItsDownloadLink() {
        val a = launch(ModelImportActivity.llmIntent(ctx))
        val m = ModelFiles.LLM_RECOMMENDATIONS.first()
        val row = rowTitled(a, m.file)
        assertTrue(row.createAccessibilityNodeInfo().actionList.any { it.label == "Copy download link" })
        row.performClick()
        val clip = ctx.getSystemService(ClipboardManager::class.java).primaryClip!!
        assertEquals(m.url, clip.getItemAt(0).text.toString())
        assertNotNull(walk(a.window.decorView).firstOrNull { it.contentDescription == "Copy download link for ${m.file}" })
    }

    @Test fun parakeetScreenImportsAFolder() {
        val a = launch(ModelImportActivity.parakeetIntent(ctx))
        val t = texts(a)
        assertTrue("Import model folder…" in t)
        assertTrue("Parakeet-TDT via sherpa-onnx" in t)
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Import model folder…" }.performClick()
        assertEquals(Intent.ACTION_OPEN_DOCUMENT_TREE, shadowOf(a).nextStartedActivityForResult.intent.action)
    }

    @Test fun importedModelShowsUseAndDeleteAsksFirst() {
        val f = File(app.engine.modelsDir(), "ggml-test.bin").apply { parentFile?.mkdirs(); writeBytes(ByteArray(2_000_000)) }
        app.settings.modelPath = f.absolutePath
        val a = launch()
        val t = texts(a)
        assertTrue("ggml-test.bin" in t)
        assertTrue("2 MB · In use" in t)
        assertTrue("Size" in t)
        val use = walk(a.window.decorView).first { it.contentDescription == "Use ggml-test.bin" }
        assertFalse("already in use", use.isEnabled)
        walk(a.window.decorView).first { it.contentDescription == "Delete ggml-test.bin" }.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d)
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertTrue("cancel keeps it", f.exists())
        walk(a.window.decorView).first { it.contentDescription == "Delete ggml-test.bin" }.performClick()
        ShadowAlertDialog.getLatestAlertDialog().getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertFalse(f.exists())
        assertNull(app.settings.modelPath)
        assertTrue("None imported yet" in texts(a))
    }

    @Test fun rejectionIsShownInlineUnderImport() {
        val a = launch(ModelImportActivity.llmIntent(ctx))
        val uri = android.net.Uri.parse("content://test/ggml-base.bin")
        shadowOf(ctx.contentResolver).registerInputStreamSupplier(uri) { java.io.ByteArrayInputStream("lmgg-whisper".toByteArray(Charsets.ISO_8859_1)) }
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Import model file…" }.performClick()
        val pick = shadowOf(a).nextStartedActivityForResult.intent
        shadowOf(a).receiveResult(pick, android.app.Activity.RESULT_OK, Intent().setData(uri))
        // Same message ModelFiles.rejection gives for a whisper file on the AI cleanup screen; nothing is copied.
        val msg = ModelFiles.rejection(ModelFiles.Kind.LLM_GGUF, "lmgg".toByteArray(Charsets.ISO_8859_1))!!
        assertTrue(texts(a).toString(), msg in texts(a))
        assertTrue(app.engine.llmDir().listFiles().orEmpty().isEmpty())
    }
}
