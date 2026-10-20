package dev.flowclone.ui

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import dev.flowclone.FlowApp
import java.io.File
import kotlin.concurrent.thread

/**
 * Model import via the Storage Access Framework (no download, no INTERNET): the user
 * picks a whisper.cpp ggml file (e.g. ggml-base-q5_1.bin) they copied to the phone; we
 * copy it into app-private `filesDir/models/` and point the engine at it.
 */
class ModelImportActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private var busy = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        render()
    }

    private fun render(status: String? = null) {
        page {
            heading("Speech model")
            body("Copy a whisper.cpp model file (ggml-*.bin, e.g. ggml-base-q5_1.bin, ~60 MB) to this phone, then import it here. Flow Clone never downloads anything.")
            status?.let { body(it) }
            val current = app.settings.modelPath
            body("Active: ${current?.let { File(it).name } ?: "none"}", if (current != null) OK_COLOR else BAD_COLOR)
            button(if (busy) "Importing…" else "Import model file…") { if (!busy) pick() }
            heading("Imported models")
            val files = app.engine.modelsDir().listFiles()?.filter { it.isFile }?.sortedBy { it.name }.orEmpty()
            if (files.isEmpty()) body("None yet.")
            for (f in files) {
                val active = f.absolutePath == current
                body("${f.name} — ${f.length() / 1_000_000} MB${if (active) " (active)" else ""}")
                row(
                    android.widget.Button(context).apply { text = "Use"; isAllCaps = false; isEnabled = !active; setOnClickListener { activate(f) } },
                    android.widget.Button(context).apply {
                        text = "Delete"; isAllCaps = false
                        setOnClickListener {
                            if (active) { app.settings.modelPath = null; reloadEngine() }
                            f.delete(); render()
                        }
                    },
                )
            }
        }
    }

    private fun pick() {
        @Suppress("DEPRECATION")
        startActivityForResult(
            Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*"),
            REQ_PICK,
        )
    }

    @Deprecated("platform Activity result API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data
        if (requestCode == REQ_PICK && resultCode == RESULT_OK && uri != null) importFrom(uri)
    }

    private fun displayName(uri: Uri): String =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        } ?: "model.bin"

    private fun importFrom(uri: Uri) {
        val name = displayName(uri).replace(Regex("[^A-Za-z0-9._-]"), "_")
        val dest = File(app.engine.modelsDir(), name)
        val tmp = File(dest.parentFile, "$name.part")
        busy = true
        render("Copying $name…")
        thread(name = "model-import") {
            val result = runCatching {
                contentResolver.openInputStream(uri)!!.use { input -> tmp.outputStream().use { input.copyTo(it, 1 shl 20) } }
                require(tmp.length() > 1_000_000) { "File is too small to be a speech model." }
                check(tmp.renameTo(dest)) { "Could not save the model." }
            }
            runOnUiThread {
                busy = false
                result.onSuccess { activate(dest) }.onFailure {
                    tmp.delete()
                    render("Import failed: ${it.message}")
                }
            }
        }
    }

    private fun activate(f: File) {
        app.settings.modelPath = f.absolutePath
        reloadEngine()
        Toast.makeText(this, "Using ${f.name}", Toast.LENGTH_SHORT).show()
        render()
    }

    private fun reloadEngine() = app.engine.reload()

    companion object { private const val REQ_PICK = 10 }
}
