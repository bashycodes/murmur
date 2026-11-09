package dev.murmur.server

import dev.murmur.data.HebrewModels
import org.json.JSONObject

/**
 * D41: installs one library model ([Plan]) through a [LibraryTransport], file by file. Each file
 * streams to `<name>.part` and appears under its name only once its size and SHA-256 match (the
 * pin for a catalogue copy, the server's listing for a custom model). An interruption (network,
 * server gone, stall) keeps the `.part`: the next [install] resumes it and skips finished files
 * (re-hashed, no bytes). A cancel or a fingerprint mismatch deletes the `.part` (nothing is kept
 * of that file). A custom Hebrew model gets [HebrewModels.ENGINE_FILE] once every file verified.
 * Blocking: run it on a worker thread.
 */
class ServerInstaller(private val transport: LibraryTransport) {
    sealed interface Outcome {
        data class Done(val plan: Plan) : Outcome
        /** [resumable]: a `.part` was kept, so another try continues where this one stopped. */
        data class Failed(val failure: LibraryFailure, val resumable: Boolean) : Outcome
        data object Cancelled : Outcome
    }

    /** [onProgress] gets (bytes on disk for the whole model, total bytes). */
    fun install(model: LibraryModel, plan: Plan, onProgress: (Long, Long) -> Unit, cancelled: () -> Boolean): Outcome {
        val total = plan.bytes
        var before = 0L
        for (f in plan.fetches) {
            if (cancelled()) return Outcome.Cancelled
            f.dest.parentFile?.mkdirs()
            val failure = transport.download(model.id, f.file, f.dest, f.part, { n -> onProgress(before + n, total) }, cancelled)
            if (failure != null) {
                if (failure.kind == "cancelled") return Outcome.Cancelled
                return Outcome.Failed(failure, resumable = failure.kind == "network" && f.part.isFile)
            }
            before += f.file.size
            onProgress(before, total)
        }
        if (plan.catalogue == null && (plan.slot == Slot.HEBREW_ZIPFORMER || plan.slot == Slot.HEBREW_PARAKEET)) {
            val engine = if (plan.slot == Slot.HEBREW_PARAKEET) "parakeet" else "sherpa_online"
            runCatching {
                java.io.File(plan.target, HebrewModels.ENGINE_FILE)
                    .writeText(JSONObject().put("engine", engine).put("title", model.title).put("server_id", model.id).toString())
            }.onFailure { return Outcome.Failed(LibraryFailure("io", "could not save the model folder: ${it.message}"), resumable = false) }
        }
        return Outcome.Done(plan)
    }
}
