package dev.murmur.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.data.HebrewImport
import dev.murmur.data.HebrewModels
import dev.murmur.data.ModelFiles
import dev.murmur.data.ModelPages
import dev.murmur.server.ServerModels
import java.io.File
import kotlin.concurrent.thread

/**
 * Model import in the Settings look (ui/DevUi, [ModelPages]): status card (active model, size,
 * engine), Import as the primary action with its result / rejection right under it, recommended
 * files with copyable download links, and the imported list with Use / Delete (asks first).
 * Model import via the Storage Access Framework (no download, no INTERNET): the user picks
 * a file they copied to the phone; we sniff its magic (ggml vs GGUF), copy it into
 * app-private storage and point the engine at it. Two kinds share this screen:
 * - speech (whisper.cpp ggml-*.bin) → `files/models/`;
 * - AI cleanup (llama.cpp *.gguf) → `files/llm/`.
 * D41: hybrid / remote builds add "From your server" ([ServerModelsSection]): the same slots,
 * downloaded from the set server's library on a tap. The local build has no such block.
 */
class ModelImportActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private var busy = false
    private val llm get() = intent.getStringExtra(EXTRA_KIND) == KIND_LLM
    private val kind get() = if (llm) ModelFiles.Kind.LLM_GGUF else ModelFiles.Kind.ASR_GGML
    private fun dir() = if (llm) app.engine.llmDir() else app.engine.modelsDir()
    private var activePath: String?
        get() = if (llm) app.settings.llmModelPath else app.settings.modelPath
        set(v) { if (llm) app.settings.llmModelPath = v else app.settings.modelPath = v }

    private val parakeet get() = intent.getStringExtra(EXTRA_KIND) == KIND_PARAKEET
    /** H5: the Hebrew model slot (a folder or .zip of a pinned model, [HebrewModels]). */
    private val hebrew get() = intent.getStringExtra(EXTRA_KIND) == KIND_HEBREW
    /** Folder-based screens (Parakeet, Hebrew). */
    private val folder get() = parakeet || hebrew
    private lateinit var ui: DevUi
    private var server: ServerModelsSection? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = if (llm) "AI cleanup model" else if (parakeet) "Parakeet model" else if (hebrew) "Hebrew model" else "Speech model"
        ui = DevUi(this)
        server = ServerModels.section(serverHost)
        render()
    }

    override fun onDestroy() {
        server?.detach()
        super.onDestroy()
    }

    private val currentPath get() = if (parakeet) app.settings.parakeetPath else if (hebrew) app.settings.hebrewModelPath else activePath

    /** D41: the server block points this screen's slot at what it downloaded, as an import would. */
    private val serverHost = object : ServerModelsHost {
        override val activity get() = this@ModelImportActivity
        override val ui get() = this@ModelImportActivity.ui
        override val screen get() = this@ModelImportActivity.screen
        override fun activePath() = currentPath
        override fun use(target: File) = if (parakeet) activateParakeet(target) else if (hebrew) activateHebrew(target) else activate(target)
        override fun refresh() = render()
    }

    private val screen get() = if (llm) ModelPages.Screen.LLM else if (parakeet) ModelPages.Screen.PARAKEET else if (hebrew) ModelPages.Screen.HEBREW else ModelPages.Screen.WHISPER
    private var statusLine: DevUi.Result? = null
    private var importButton: android.widget.Button? = null

    /** Rebuilds the page (keeps the scroll position); [status] = the line under Import. */
    private fun render(status: String? = null, tone: Tone = Tone.NEUTRAL): Unit = with(ui) {
        val current = currentPath
        val items = if (parakeet) {
            app.engine.parakeetDir().listFiles()?.filter { !it.name.endsWith(".part") && dev.murmur.data.ParakeetFiles.isModelDir(it) }?.sortedBy { it.name }.orEmpty()
        } else if (hebrew) {
            app.engine.hebrewDir().listFiles()?.filter { !it.name.endsWith(".part") && HebrewModels.installed(it) != null }?.sortedBy { it.name }.orEmpty()
        } else dir().listFiles()?.filter { it.isFile && !it.name.endsWith(".part") }?.sortedBy { it.name }.orEmpty()
        fun bytes(f: File) = if (f.isDirectory) f.walk().filter { it.isFile }.sumOf { it.length() } else f.length()
        rebuild {
            topBar(title.toString(), DevUi.TopNav.BACK, onNav = ::finish)
            note(ModelPages.intro(screen, server != null))
            sectionLabel("STATUS")
            card {
                val active = current?.let(::File)
                infoRow("Active model", ModelPages.active(active?.name)).setTextColor(if (active != null) p.ok else p.danger)
                if (active != null && active.exists()) infoRow("Size", ModelPages.size(bytes(active)))
                infoRow("Engine", ModelPages.engine(screen))
            }
            importButton = action(ModelPages.importLabel(screen, busy), DevUi.Style.PRIMARY) {
                @Suppress("DEPRECATION")
                if (busy) Unit else if (folder) startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), REQ_TREE) else pick()
            }
            if (hebrew) action("Import .zip…", DevUi.Style.TONAL) {
                @Suppress("DEPRECATION")
                if (!busy) startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/zip"), REQ_ZIP)
            }
            statusLine = result().also { r -> status?.let { r.show(it, tone) } }
            server?.render(this)

            sectionLabel("RECOMMENDED")
            card {
                for (m in ModelPages.recommended(screen)) {
                    val copyLink = { copyLink(m.file, m.url) }
                    listRow(
                        m.file, ModelPages.subtitle(m),
                        trailing = iconButton(R.drawable.ic_scr_copy, "Copy download link for ${m.file}") { copyLink() },
                        actions = listOf("Copy download link" to copyLink),
                    ) { copyLink() }
                    selectableText(m.url, 12f, p.muted)
                }
            }
            note(ModelPages.recommendedNote(folder, server != null))

            sectionLabel("IMPORTED")
            card {
                if (items.isEmpty()) emptyState("None imported yet", "Imported models are copied into the app's private storage.")
                for (f in items) {
                    val active = f.absolutePath == current
                    stateRow(if (folder) R.drawable.ic_scr_folder else R.drawable.ic_scr_file, f.name, ModelPages.imported(bytes(f), active), if (active) "In use" else "", if (active) Tone.OK else Tone.NEUTRAL)
                    chipRow(
                        chip("Use") { if (parakeet) activateParakeet(f) else if (hebrew) activateHebrew(f) else activate(f) }.apply { isEnabled = !active; alpha = if (active) 0.5f else 1f; contentDescription = "Use ${f.name}" },
                        chip("Delete") { confirmDelete(f, active) }.apply { setTextColor(p.danger); contentDescription = "Delete ${f.name}" },
                    )
                }
            }
        }
    }

    /** Progress while copying: only the status line and the button change. */
    private fun progress(text: String) {
        statusLine?.show(text) ?: render(text)
        importButton?.text = ModelPages.importLabel(screen, busy)
    }

    private fun copyLink(file: String, url: String) {
        getSystemService(android.content.ClipboardManager::class.java)?.setPrimaryClip(android.content.ClipData.newPlainText("Download link for $file", url))
        Toast.makeText(this, "Download link copied", Toast.LENGTH_SHORT).show()
        ui.announce("Download link copied")
    }

    private fun confirmDelete(f: File, active: Boolean) {
        ui.confirm(
            "Delete ${f.name}?",
            "It is removed from the app's storage" + if (active) " and the engine reloads without it." else ".",
            "Delete",
            onYes = {
                if (parakeet) {
                    if (active) { app.settings.parakeetPath = null; reloadEngine() }
                    f.deleteRecursively()
                } else if (hebrew) {
                    if (active) { app.settings.hebrewModelPath = null; reloadEngine() }
                    f.deleteRecursively()
                } else {
                    if (active) { activePath = null; reloadEngine() }
                    f.delete()
                }
                ui.announce("Deleted ${f.name}")
                render()
            },
        )
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
        if (requestCode == REQ_TREE && resultCode == RESULT_OK && uri != null) { if (hebrew) importHebrewTree(uri) else importTree(uri) }
        if (requestCode == REQ_ZIP && resultCode == RESULT_OK && uri != null) importHebrewZip(uri)
    }

    private fun meta(uri: Uri): Pair<String, Long> =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) (c.getString(0) ?: "model.bin") to (if (c.isNull(1)) -1L else c.getLong(1)) else null
        } ?: ("model.bin" to -1L)

    private fun importFrom(uri: Uri) {
        val (rawName, size) = meta(uri)
        val name = rawName.replace(Regex("[^A-Za-z0-9._-]"), "_")
        val header = runCatching { contentResolver.openInputStream(uri)!!.use { ModelFiles.readHeader(it) } }.getOrNull()
        ModelFiles.rejection(kind, header)?.let { render(it, Tone.BAD); return }
        val dest = File(dir(), name)
        val tmp = File(dest.parentFile, "$name.part")
        if (size > 0 && dir().usableSpace < size + 50_000_000) {
            render("Not enough free space: the file is ${size / 1_000_000} MB, ${dir().usableSpace / 1_000_000} MB free.", Tone.BAD)
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
                                runOnUiThread { progress("Copying $name… ${total / 1_000_000} MB$pct") }
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
                    render("Import failed: ${it.message}", Tone.BAD)
                }
            }
        }
    }

    // ---- Parakeet: a sherpa-onnx model *folder* (encoder/decoder/joiner *.onnx + tokens.txt) ----

    /** The picked folder's direct children: display name → (document uri, size or -1). */
    private fun children(tree: Uri): Map<String, Pair<Uri, Long>> {
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
        return entries
    }

    // ---- Hebrew (H5): a pinned model's four files, from a folder or a .zip, hash-checked ----

    private fun importHebrewTree(tree: Uri) {
        val entries = children(tree)
        val pre = HebrewModels.identify(entries.mapValues { HebrewModels.Facts(it.value.second) })
        if (pre is HebrewModels.Check.Bad) return render(pre.message, Tone.BAD)
        val total = HebrewModels.FILES.sumOf { entries.getValue(it).second.coerceAtLeast(0) }
        importHebrew(total) { tmp, onBytes ->
            HebrewModels.FILES.associateWith { name ->
                contentResolver.openInputStream(entries.getValue(name).first)!!.use { HebrewImport.copyHashed(it, File(tmp, name), onBytes) }
            }
        }
    }

    private fun importHebrewZip(uri: Uri) {
        val (_, size) = meta(uri)
        importHebrew(size) { tmp, onBytes ->
            contentResolver.openInputStream(uri)!!.use { HebrewImport.extractZip(it, tmp, onBytes) }
        }
    }

    /** Copies into `hebrew/.import.part`, checks sizes + SHA-256, then moves to `hebrew/<kind>/` and uses it. */
    private fun importHebrew(totalBytes: Long, copy: (File, (Long) -> Unit) -> Map<String, HebrewModels.Facts>) {
        val root = app.engine.hebrewDir()
        if (totalBytes > 0 && root.usableSpace < totalBytes + 100_000_000) return render("Not enough free space: needs ${totalBytes / 1_000_000} MB.", Tone.BAD)
        busy = true
        render("Copying and checking the Hebrew model…")
        thread(name = "hebrew-import") {
            val tmp = File(root, ".import.part")
            val result = runCatching {
                tmp.deleteRecursively(); tmp.mkdirs()
                var done = 0L; var shown = 0L
                val facts = copy(tmp) { n ->
                    done += n
                    if (done - shown >= 16_000_000) {
                        shown = done
                        val of = if (totalBytes > 0) " / ${totalBytes / 1_000_000}" else ""
                        runOnUiThread { progress("Copying… ${done / 1_000_000}$of MB") }
                    }
                }
                when (val c = HebrewModels.identify(facts)) {
                    is HebrewModels.Check.Bad -> error(c.message)
                    is HebrewModels.Check.Ok -> {
                        val dest = File(root, c.kind.dirName)
                        dest.deleteRecursively()
                        check(tmp.renameTo(dest)) { "Could not save the model folder." }
                        dest
                    }
                }
            }
            runOnUiThread {
                busy = false
                result.onSuccess { activateHebrew(it) }.onFailure {
                    app.debugLog.event("model_import", "hebrew: ${it.message}", it)
                    tmp.deleteRecursively()
                    render("Import failed: ${it.message}", Tone.BAD)
                }
            }
        }
    }

    private fun activateHebrew(d: File) {
        app.settings.hebrewModelPath = d.absolutePath
        val kind = HebrewModels.installed(d)
        // Reload only when the next dictation is Hebrew (it then loads this model); otherwise nothing to do yet.
        if (HebrewModels.isHebrew(app.effectiveLanguage())) reloadEngine()
        Toast.makeText(this, "Hebrew dictation uses ${kind?.label ?: d.name}.", Toast.LENGTH_LONG).show()
        render()
    }

    private fun importTree(tree: Uri) {
        val docId = android.provider.DocumentsContract.getTreeDocumentId(tree)
        val entries = children(tree)
        val pick = dev.murmur.data.ParakeetFiles.select(entries.keys)
            ?: return render("Not a Parakeet model folder; missing ${dev.murmur.data.ParakeetFiles.missing(entries.keys).joinToString()}.", Tone.BAD)
        val folder = (docId.substringAfterLast('/').substringAfterLast(':').ifEmpty { "parakeet" }).replace(Regex("[^A-Za-z0-9._-]"), "_")
        val dest = File(app.engine.parakeetDir(), folder)
        val tmp = File(app.engine.parakeetDir(), "$folder.part")
        val total = pick.values.sumOf { entries.getValue(it).second.coerceAtLeast(0) }
        if (app.engine.parakeetDir().usableSpace < total + 100_000_000) return render("Not enough free space: needs ${total / 1_000_000} MB.", Tone.BAD)
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
                                if (done % 64_000_000 < n) runOnUiThread { progress("Copying… ${done / 1_000_000} / ${total / 1_000_000} MB") }
                            }
                        }
                    }
                }
                dest.deleteRecursively()
                check(tmp.renameTo(dest)) { "Could not save the model folder." }
            }
            runOnUiThread {
                busy = false
                result.onSuccess { activateParakeet(dest) }.onFailure { app.debugLog.event("model_import", "parakeet: ${it.message}", it); tmp.deleteRecursively(); render("Import failed: ${it.message}", Tone.BAD) }
            }
        }
    }

    private fun activateParakeet(d: File) {
        app.settings.parakeetPath = d.absolutePath
        val s = app.settings.load()
        if (s.asrEngine != dev.murmur.core.AsrEngine.PARAKEET) app.settings.save(s.copy(asrEngine = dev.murmur.core.AsrEngine.PARAKEET))
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
        private const val REQ_ZIP = 12
        const val KIND_PARAKEET = "parakeet"
        const val KIND_HEBREW = "hebrew"
        fun hebrewIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_HEBREW)
        fun parakeetIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_PARAKEET)
        const val EXTRA_KIND = "kind"
        const val KIND_LLM = "llm"

        fun llmIntent(c: Context) = Intent(c, ModelImportActivity::class.java).putExtra(EXTRA_KIND, KIND_LLM)
    }
}
