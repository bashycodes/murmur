package dev.murmur.server

import org.json.JSONObject
import java.io.File

/*
 * D41 on Android: models from the set server's library (server side D40). This source set
 * (src/server) is compiled only into the hybrid / remote flavours; the local flavour has a
 * no-op stand-in (src/noserver) and no network code at all (D9).
 *
 * The HTTP client is crates/murmur-remote `library` (shared with Windows), reached through
 * murmur-ffi's `library` feature ([ServerLibraryNative], JNI_CONTRACT.md "Model library").
 */

/** What a library model is for (`murmur_protocol::library::ModelKind`); null in [LibraryModel.kind] = a kind this build doesn't know. */
enum class ModelKind(val wire: String) {
    STT("stt"), HEBREW("hebrew"), LLM("llm"), LID("lid"), VAD("vad");

    companion object {
        fun of(wire: String): ModelKind? = entries.firstOrNull { it.wire == wire }
    }
}

/** One file of a library model: the expected size and SHA-256 of a download. */
data class LibraryFile(val name: String, val size: Long, val sha256: String) {
    fun toJson(): String = JSONObject().put("name", name).put("size", size).put("sha256", sha256).toString()
}

data class LibraryModel(
    val id: String,
    val title: String,
    /** Null: a kind newer than this build (listed with the reason, can't be chosen). */
    val kind: ModelKind?,
    val kindWire: String,
    val language: String? = null,
    /** The app catalogue id this model is a copy of (checked against [ServerCatalog]'s pins). */
    val catalogueId: String? = null,
    val engine: String? = null,
    val files: List<LibraryFile>,
) {
    val size: Long get() = files.sumOf { it.size }
}

/** A failed library call: [kind] is the JNI_CONTRACT kind (`network`, `auth`, `mismatch`, `cancelled`, …). */
data class LibraryFailure(val kind: String, val message: String)

sealed interface ListResult {
    data class Ok(val models: List<LibraryModel>) : ListResult
    data class Failed(val failure: LibraryFailure) : ListResult
}

object LibraryJson {
    private fun failure(o: JSONObject) =
        LibraryFailure(o.optString("error").ifEmpty { "protocol" }, o.optString("message").ifEmpty { "the server's answer could not be read" })

    private fun str(o: JSONObject, k: String): String? = if (o.isNull(k)) null else o.optString(k).ifEmpty { null }

    fun parseList(json: String?): ListResult {
        val o = runCatching { JSONObject(json ?: "") }.getOrNull()
            ?: return ListResult.Failed(LibraryFailure("protocol", "the model list could not be read"))
        if (!o.optBoolean("ok")) return ListResult.Failed(failure(o))
        val arr = o.optJSONArray("models") ?: return ListResult.Ok(emptyList())
        val models = (0 until arr.length()).mapNotNull { i ->
            val m = arr.optJSONObject(i) ?: return@mapNotNull null
            val files = m.optJSONArray("files") ?: return@mapNotNull null
            LibraryModel(
                id = m.optString("id"),
                title = m.optString("title").ifEmpty { m.optString("id") },
                kind = ModelKind.of(m.optString("kind")),
                kindWire = m.optString("kind"),
                language = str(m, "language"),
                catalogueId = str(m, "catalogue_id"),
                engine = str(m, "engine"),
                files = (0 until files.length()).mapNotNull { j ->
                    files.optJSONObject(j)?.let { f -> LibraryFile(f.optString("name"), f.optLong("size"), f.optString("sha256")) }
                },
            ).takeIf { it.id.isNotEmpty() && it.files.isNotEmpty() }
        }
        return ListResult.Ok(models)
    }

    /** `nativeDownload`'s result: null = ok. */
    fun parseDownload(json: String?): LibraryFailure? {
        val o = runCatching { JSONObject(json ?: "") }.getOrNull()
            ?: return LibraryFailure("protocol", "the download result could not be read")
        return if (o.optBoolean("ok")) null else failure(o)
    }
}

/** The library calls the installer needs (a fake in tests; [NativeLibraryTransport] in the app). */
interface LibraryTransport {
    fun list(): ListResult

    /**
     * One file into [dest] through [part] (resumable, verified against [file]). [onProgress] gets
     * the bytes of this file on disk; [cancelled] is polled between chunks. Null = ok.
     */
    fun download(modelId: String, file: LibraryFile, dest: File, part: File, onProgress: (Long) -> Unit, cancelled: () -> Boolean): LibraryFailure?
}

/** murmur-ffi's `library` natives with the server config JSON ([dev.murmur.data.DevSettings.toServerConfigJson]). */
class NativeLibraryTransport(private val serverConfigJson: String) : LibraryTransport {
    private fun linked(): LibraryFailure? = when (val s = dev.murmur.core.NativeMurmurCore.loadLibrary()) {
        is dev.murmur.core.NativeMurmurCore.LibraryState.Loaded -> null
        is dev.murmur.core.NativeMurmurCore.LibraryState.Unavailable -> LibraryFailure("protocol", s.message)
    }

    override fun list(): ListResult {
        linked()?.let { return ListResult.Failed(it) }
        return try {
            LibraryJson.parseList(ServerLibraryNative.nativeList(serverConfigJson))
        } catch (e: UnsatisfiedLinkError) {
            ListResult.Failed(OLD_LIBRARY)
        }
    }

    override fun download(modelId: String, file: LibraryFile, dest: File, part: File, onProgress: (Long) -> Unit, cancelled: () -> Boolean): LibraryFailure? {
        linked()?.let { return it }
        val listener = object : ServerLibraryNative.Listener {
            override fun onProgress(bytesOnDisk: Long) = onProgress(bytesOnDisk)
            override fun isCancelled(): Boolean = cancelled()
        }
        return try {
            LibraryJson.parseDownload(ServerLibraryNative.nativeDownload(serverConfigJson, modelId, file.toJson(), dest.path, part.path, listener))
        } catch (e: UnsatisfiedLinkError) {
            OLD_LIBRARY
        }
    }

    private companion object {
        val OLD_LIBRARY = LibraryFailure("protocol", "This build's native library has no server downloads (rebuild it with tools/build-native.sh).")
    }
}

/** JNI_CONTRACT.md "Model library": exported by libmurmur_ffi.so only with cargo feature `library`. */
object ServerLibraryNative {
    /** Called on the downloading thread. */
    interface Listener {
        fun onProgress(bytesOnDisk: Long)
        fun isCancelled(): Boolean
    }

    @JvmStatic external fun nativeList(server: String): String
    @JvmStatic external fun nativeDownload(server: String, modelId: String, file: String, dest: String, part: String, listener: Any?): String
}
