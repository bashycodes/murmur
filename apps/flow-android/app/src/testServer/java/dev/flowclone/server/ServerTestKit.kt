package dev.flowclone.server

import java.io.File
import java.security.MessageDigest

internal fun sha(b: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

internal fun tempDirs(): Pair<File, ModelDirs> {
    val root = kotlin.io.path.createTempDirectory("srvlib").toFile()
    return root to ModelDirs(File(root, "models"), File(root, "parakeet"), File(root, "hebrew"), File(root, "llm"))
}

internal fun model(
    id: String, kind: ModelKind?, engine: String?, files: List<LibraryFile>,
    catalogueId: String? = null, title: String = id,
) = LibraryModel(id, title, kind, kind?.wire ?: "newkind", null, catalogueId, engine, files)

internal fun f(name: String, size: Long = 10, hash: String = "a".repeat(64)) = LibraryFile(name, size, hash)

internal val SHERPA = listOf(f("encoder.int8.onnx"), f("decoder.int8.onnx"), f("joiner.int8.onnx"), f("tokens.txt"))

/**
 * flow_remote::library's contract in miniature: bytes go to `part`; a cut-off keeps `part` and
 * the next call continues from its length; a hash mismatch or a cancel deletes it; `dest`
 * appears only verified; an intact `dest` is kept without a "request".
 */
internal class FakeTransport(private val bytes: Map<String, ByteArray>) : LibraryTransport {
    var cutAfter: Long? = null
    val requests = mutableListOf<Pair<String, Long>>()

    override fun list() = ListResult.Ok(emptyList())

    override fun download(modelId: String, file: LibraryFile, dest: File, part: File, onProgress: (Long) -> Unit, cancelled: () -> Boolean): LibraryFailure? {
        if (dest.isFile && dest.length() == file.size && sha(dest.readBytes()) == file.sha256) { onProgress(file.size); return null }
        val src = bytes[file.name] ?: return LibraryFailure("not_found", "not on the server")
        if (sha(src) != file.sha256 || src.size.toLong() != file.size) { part.delete(); return LibraryFailure("mismatch", "${file.name} differs") }
        part.parentFile?.mkdirs()
        val have = if (part.isFile && part.length() <= file.size) part.length() else 0L
        requests += file.name to have
        var at = have
        java.io.RandomAccessFile(part, "rw").use { out ->
            out.setLength(have); out.seek(have)
            while (at < file.size) {
                if (cancelled()) { out.close(); part.delete(); return LibraryFailure("cancelled", "cancelled") }
                val cut = cutAfter
                if (cut != null && at >= cut) { cutAfter = null; return LibraryFailure("network", "${file.name} stopped") }
                out.write(src, at.toInt(), 1); at++
                onProgress(at)
            }
        }
        if (sha(part.readBytes()) != file.sha256) { part.delete(); return LibraryFailure("mismatch", "fingerprint") }
        dest.parentFile?.mkdirs()
        check(part.renameTo(dest))
        return null
    }
}
