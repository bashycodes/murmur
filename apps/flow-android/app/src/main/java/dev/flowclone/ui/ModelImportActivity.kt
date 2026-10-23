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

    private val parakeet get() = intent.getStringExtra(EXTRA_KIND) == KIND_PARAKEET

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = if (llm) "AI cleanup model" else if (parakeet) "Parakeet model" else "Speech model"
        render()
    }

    private fun render(status: String? = null) {
        if (parakeet) return renderParakeet(status)
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
        if (requestCode == REQ_TREE && resultCode == RESULT_OK && uri != null) importTree(uri)
    }

    private fun meta(uri: Uri): Pair<String, Long> =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) (c.getString(0) ?: "model.bin") to (if (c.isNull(1)) -1L else c.getLong(1)) else null
        } ?: ("model.bin" to -1L)

    private fun importFrom(uri: Uri) {
        val (rawName, size) = meta(uri)
        val name = rawName.replace(Regex("[^A-Za-z0-9._-]"), "_")
        val header = runCatching { contentResolver.openInputStream(uri)!!.use { ModelFiles.readHeader(it) } }.getOrNull()
        ModelFiles.rejection(kind, header)?.let { render(it); return }
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
                    app.debugLog.event("model_import", "whisper: ${it.message}", it)
                    tmp.delete()
                    render("Import failed: ${it.message}")
                }
            }
        }
    }

    // ---- Parakeet: a sherpa-onnx model *folder* (encoder/decoder/joiner *.onnx + tokens.txt) ----

    private fun renderParakeet(status: String?) {
        page {
            heading("Parakeet-TDT 0.6B v2 (int8)")
            body(
                "English only, more accurate than whisper base (WER 1.2% vs 3.7% on the desktop bench). Copy the folder " +
                    "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 (~661 MB: encoder.int8.onnx, decoder.int8.onnx, joiner.int8.onnx, tokens.txt) " +
                    "to the phone, then pick that folder here. Needs ~1 GB free RAM; recommended on phones with 6-8 GB.",
            )
            status?.let { body(it) }
            val current = app.settings.parakeetPath
            body("Active: ${current?.let { File(it).name } ?: "none"}", if (current != null) OK_COLOR else BAD_COLOR)
            button(if (busy) "Importing…" else "Import model folder…") {
                @Suppress("DEPRECATION")
                if (!busy) startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), REQ_TREE)
            }
            heading("Imported")
            val dirs = app.engine.parakeetDir().listFiles()?.filter { !it.name.endsWith(".part") && dev.flowclone.data.ParakeetFiles.isModelDir(it) }?.sortedBy { it.name }.orEmpty()
            if (dirs.isEmpty()) body("None yet.")
            for (d in dirs) {
                val active = d.absolutePath == current
                body("${d.name} — ${d.walk().filter { it.isFile }.sumOf { it.length() } / 1_000_000} MB${if (active) " (active)" else ""}")
                row(
                    Button(context).apply { text = "Use"; isAllCaps = false; isEnabled = !active; setOnClickListener { activateParakeet(d) } },
                    Button(context).apply {
                        text = "Delete"; isAllCaps = false
                        setOnClickListener {
                            if (active) { app.settings.parakeetPath = null; reloadEngine() }
                            d.deleteRecursively(); render()
                        }
                    },
                )
            }
        }
    }

    private fun importTree(tree: Uri) {
        val docId = android.provider.DocumentsContract.getTreeDocumentId(tree)
        val children = android.provider.DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId)
        val entries = HashMap<String, Pair<Uri, Long>>()
        contentResolver.query(
            children,
            arrayOf(android.provider.DocumentsContract.Document.COLUMN_DOCUMENT_ID, android.provider.DocumentsContract.Document.COLUMN_DISPLAY_NAME, android.provider.DocumentsContract.Document.COLUMN_SIZE),
            null, null, null,
        )?.use { c ->
            while (c.moveToNext()) {
                val name = c.getString(1) ?: continue
                entries[name] = android.provider.DocumentsContract.buildDocumentUriUsingTree(tree, c.getString(0)) to (if (c.isNull(2)) -1L else c.getLong(2))
            }
        }
        val pick = dev.flowclone.data.ParakeetFiles.select(entries.keys)
            ?: return render("Not a Parakeet model folder; missing ${dev.flowclone.data.ParakeetFiles.missing(entries.keys).joinToString()}.")
        val folder = (docId.substringAfterLast('/').substringAfterLast(':').ifEmpty { "parakeet" }).replace(Regex("[^A-Za-z0-9._-]"), "_")
        val dest = File(app.engine.parakeetDir(), folder)
        val tmp = File(app.engine.parakeetDir(), "$folder.part")
        val total = pick.values.sumOf { entries.getValue(it).second.coerceAtLeast(0) }
        if (app.engine.parakeetDir().usableSpace < total + 100_000_000) return render("Not enough free space: needs ${total / 1_000_000} MB.")
        busy = true
        render("Copying ${pick.size} files (${total / 1_000_000} MB)…")
        thread(name = "parakeet-import") {
            val result = runCatching {
                tmp.deleteRecursively(); tmp.mkdirs()
                var done = 0L
                for (name in pick.values) {
                    contentResolver.openInputStream(entries.getValue(name).first)!!.use { input ->
                        File(tmp, name).outputStream().use { out ->
                            val buf = ByteArray(1 shl 20)
                            while (true) {
                                val n = input.read(buf); if (n < 0) break
                                out.write(buf, 0, n); done += n
                                if (done % 64_000_000 < n) runOnUiThread { render("Copying… ${done / 1_000_000} / ${total / 1_000_000} MB") }
                            }
                        }
                    }
                }
                dest.deleteRecursively()
                check(tmp.renameTo(dest)) { "Could not save the model folder." }
            }
            runOnUiThread {
                busy = false
                result.onSuccess { activateParakeet(dest) }.onFailure { app.debugLog.event("model_import", "parakeet: ${it.message}", it); tmp.deleteRecursively(); render("Import failed: ${it.message}") }
            }
        }
    }

    private fun activateParakeet(d: File) {
        app.settings.parakeetPath = d.absolutePath
        val s = app.settings.load()
        if (s.asrEngine != dev.flowclone.core.AsrEngine.PARAKEET) app.settings.save(s.copy(asrEngine = dev.flowclone.core.AsrEngine.PARAKEET))
        reloadEngine()
        Toast.makeText(this, "Using Parakeet (${d.name}).", Toast.LENGTH_LONG).show()
        render()
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
        private const val REQ_TREE = 11
        const val KIND_PARAKEET = "parakeet"
        fun parakeetIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_PARAKEET)
        const val EXTRA_KIND = "kind"
        const val KIND_LLM = "llm"

        fun llmIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_LLM)
    }
}
