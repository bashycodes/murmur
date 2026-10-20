package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Button
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.data.ModelFiles
import java.io.File
import kotlin.concurrent.thread

/**
 * Model import via the Storage Access Framework (no download, no INTERNET): the user picks
 * a file they copied to the phone; we sniff its magic (ggml vs GGUF), copy it into
 * app-private storage and point the engine at it. Two kinds share this screen:
 * - speech (whisper.cpp ggml-*.bin) → `files/models/`;
 * - AI cleanup (llama.cpp *.gguf) → `files/llm/`.
 */
class ModelImportActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private var busy = false
    private val llm get() = intent.getStringExtra(EXTRA_KIND) == KIND_LLM
    private val kind get() = if (llm) ModelFiles.Kind.LLM_GGUF else ModelFiles.Kind.ASR_GGML
    private fun dir() = if (llm) app.engine.llmDir() else app.engine.modelsDir()
    private var activePath: String?
        get() = if (llm) app.settings.llmModelPath else app.settings.modelPath
        set(v) { if (llm) app.settings.llmModelPath = v else app.settings.modelPath = v }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = if (llm) "AI cleanup model" else "Speech model"
        render()
    }

    private fun render(status: String? = null) {
        page {
            if (llm) {
                heading("AI cleanup model (GGUF)")
                body(
                    "Optional. A small local language model rewrites the transcript: punctuation, filler words, " +
                        "\"scratch that\" corrections and lists. It runs only on this phone and is off until you turn it on in Settings. " +
                        "Copy one of these files to the phone (download it on a computer; this app can't), then import it:",
                )
                for (r in ModelFiles.LLM_RECOMMENDATIONS) body("• ${r.file} (~${r.sizeMb} MB): ${r.note}\n  ${r.url}")
                body("Expect 1–3 s extra per dictation on a recent phone; if cleanup takes longer than 3 s the plain transcript is inserted instead.")
            } else {
                heading("Speech model")
                body("Copy a whisper.cpp model file (ggml-*.bin, e.g. ggml-base.en-q5_1.bin, ~60 MB) to this phone, then import it here. Flow Clone never downloads anything.")
            }
            status?.let { body(it) }
            val current = activePath
            body("Active: ${current?.let { File(it).name } ?: "none"}", if (current != null) OK_COLOR else BAD_COLOR)
            button(if (busy) "Importing…" else "Import model file…") { if (!busy) pick() }
            heading("Imported")
            val files = dir().listFiles()?.filter { it.isFile && !it.name.endsWith(".part") }?.sortedBy { it.name }.orEmpty()
            if (files.isEmpty()) body("None yet.")
            for (f in files) {
                val active = f.absolutePath == current
                body("${f.name} — ${f.length() / 1_000_000} MB${if (active) " (active)" else ""}")
                row(
                    Button(context).apply { text = "Use"; isAllCaps = false; isEnabled = !active; setOnClickListener { activate(f) } },
                    Button(context).apply {
                        text = "Delete"; isAllCaps = false
                        setOnClickListener {
                            if (active) { activePath = null; reloadEngine() }
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

    private fun meta(uri: Uri): Pair<String, Long> =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) (c.getString(0) ?: "model.bin") to (if (c.isNull(1)) -1L else c.getLong(1)) else null
        } ?: ("model.bin" to -1L)

    private fun importFrom(uri: Uri) {
        val (rawName, size) = meta(uri)
        val name = rawName.replace(Regex("[^A-Za-z0-9._-]"), "_")
        val header = runCatching { contentResolver.openInputStream(uri)!!.use { s -> ByteArray(4).also { s.read(it) } } }.getOrNull()
        val mismatch = header?.let { ModelFiles.mismatch(kind, ModelFiles.kindOf(it)) } ?: "Could not read the file."
        if (mismatch != null) { render(mismatch); return }
        val dest = File(dir(), name)
        val tmp = File(dest.parentFile, "$name.part")
        if (size > 0 && dir().usableSpace < size + 50_000_000) {
            render("Not enough free space: the file is ${size / 1_000_000} MB, ${dir().usableSpace / 1_000_000} MB free.")
            return
        }
        busy = true
        render("Copying $name…")
        thread(name = "model-import") {
            val result = runCatching {
                contentResolver.openInputStream(uri)!!.use { input ->
                    tmp.outputStream().use { out ->
                        val buf = ByteArray(1 shl 20)
                        var total = 0L; var lastShown = 0L
                        while (true) {
                            val n = input.read(buf); if (n < 0) break
                            out.write(buf, 0, n); total += n
                            if (total - lastShown >= 32_000_000) {
                                lastShown = total
                                val pct = if (size > 0) " (${total * 100 / size}%)" else ""
                                runOnUiThread { render("Copying $name… ${total / 1_000_000} MB$pct") }
                            }
                        }
                    }
                }
                require(tmp.length() > 1_000_000) { "File is too small to be a model." }
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
        activePath = f.absolutePath
        reloadEngine()
        val hint = if (llm && !app.settings.load().llmEnabled) " Turn on AI cleanup in Settings to use it." else ""
        Toast.makeText(this, "Using ${f.name}.$hint", Toast.LENGTH_LONG).show()
        render()
    }

    private fun reloadEngine() = app.engine.reload()

    companion object {
        private const val REQ_PICK = 10
        const val EXTRA_KIND = "kind"
        const val KIND_LLM = "llm"

        fun llmIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_LLM)
    }
}
