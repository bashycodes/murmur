package dev.flowclone.server

import dev.flowclone.data.ModelPages

/** D41: the words of the "From your server" block (JVM-tested; the views are [ServerModelsUi]). */
object ServerModelPages {
    fun mb(bytes: Long): String = ModelPages.size(bytes)

    /** Row subtitle: size, files, where it goes or why it can't be used. */
    fun subtitle(m: LibraryModel, plan: Result<Plan>): String {
        val files = if (m.files.size == 1) "1 file" else "${m.files.size} files"
        val p = plan.getOrNull()
        val size = mb(p?.bytes ?: m.size)
        return when {
            p == null -> "$size · $files · Can't be used: ${plan.exceptionOrNull()?.message}"
            p.catalogue != null -> "$size · ${p.slot.label} · checked against the fingerprint built into this app"
            else -> "$size · $files · ${p.slot.label}"
        }
    }

    /** The row's action: Download, Resume (a `.part` or some files already here), Use, or none. */
    fun action(plan: Plan?, inUse: Boolean): String? = when {
        plan == null -> null
        plan.installed -> if (inUse) null else "Use"
        plan.bytesOnDisk > 0 -> "Resume"
        else -> "Download"
    }

    fun state(plan: Plan?, inUse: Boolean, downloading: Boolean): String = when {
        downloading -> "Downloading"
        plan == null -> "Not usable"
        inUse -> "In use"
        plan.installed -> "Downloaded"
        else -> ""
    }

    fun confirmTitle(m: LibraryModel) = "Download ${m.title}?"

    /** The confirm dialog: what is fetched, how much (the user may be on mobile data), from where, and how it is checked. */
    fun confirmMessage(m: LibraryModel, plan: Plan, host: String): String {
        val left = plan.bytes - plan.bytesOnDisk
        val amount = if (plan.bytesOnDisk > 0) "${mb(left)} left of ${mb(plan.bytes)}" else mb(plan.bytes)
        val check = if (plan.catalogue != null) "the fingerprints built into this app" else "the fingerprints your server lists"
        return "$amount from your server ($host), saved in this app's storage. On mobile data this uses about ${mb(left)} of your plan.\n\n" +
            "Every file is checked against $check before it is used. If the download stops, tap Resume to continue where it left off."
    }

    fun progress(title: String, done: Long, total: Long): String {
        val pct = if (total > 0) " (${done * 100 / total} %)" else ""
        return "Downloading $title… ${done / 1_000_000} / ${total / 1_000_000} MB$pct"
    }

    fun failed(f: LibraryFailure, resumable: Boolean): String = when {
        f.kind == "not_configured" -> "Set up your server first (Settings → Your server)."
        f.kind == "auth" -> "Your server refused the key: check it in the server settings."
        resumable -> "Download stopped: ${f.message}. What arrived is kept; tap Resume to continue."
        else -> "Download failed: ${f.message}"
    }

    fun listFailed(f: LibraryFailure): String = when (f.kind) {
        "not_configured" -> "Set up your server first (Settings → Your server)."
        "auth" -> "Your server refused the key: check it in the server settings."
        else -> "Couldn't read your server's models: ${f.message}"
    }

    fun noSpace(needBytes: Long, freeBytes: Long) = "Not enough free space: needs ${mb(needBytes)}, ${mb(freeBytes)} free."

    fun empty(screen: ModelPages.Screen) = "Your server has no ${when (screen) {
        ModelPages.Screen.WHISPER -> "whisper speech"
        ModelPages.Screen.PARAKEET -> "Parakeet"
        ModelPages.Screen.HEBREW -> "Hebrew"
        ModelPages.Screen.LLM -> "AI cleanup"
    }} models. Add one to its library folder with a model.toml tag."
}
