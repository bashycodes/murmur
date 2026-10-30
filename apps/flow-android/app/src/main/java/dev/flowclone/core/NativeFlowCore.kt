package dev.flowclone.core

/**
 * JNI binding to crates/flow-ffi (`libflow_ffi.so`). Exact symbol names and signatures:
 * apps/flow-android/JNI_CONTRACT.md — keep the two in sync.
 *
 * The library is optional at build time: [loadLibrary] reports a missing/broken .so as
 * [LibraryState.Unavailable] instead of throwing, and the UI shows that message.
 */
class NativeFlowCore private constructor(handle: Long, val isRouter: Boolean = false) : FlowCore {
    sealed interface LibraryState {
        data class Loaded(val version: String) : LibraryState
        data class Unavailable(val message: String) : LibraryState
    }

    /**
     * Review C1: every native call holds the lease, so [unload] (Settings save → reload,
     * ACTION_RELOAD_ENGINE, onTrimMemory) can't free the box under a running push/commit/poll;
     * the free is deferred to the last caller and runs on a background thread.
     */
    private val lease = HandleLease(handle, { nativeUnload(it) })

    /** Engine unloaded: a [CoreException] (review C16: the controller catches only those). */
    private fun gone(): Nothing = throw CoreException("Speech engine was unloaded, try again in a moment.", "local_not_ready")

    override fun startSession(cfg: SessionConfig) = lease.use(::gone) { h ->
        BlockingCalls.check("nativeStartSession")
        if (!nativeStartSession(h, CoreJson.encode(cfg))) throw lastError("start_session failed")
    }

    /**
     * Gives the handle away (to `nativeOpenRouter`, which takes ownership). This instance is
     * unusable afterwards and its [unload] is a no-op.
     */
    fun release(): Long = lease.release()

    /** True until [unload] / [release]. */
    val isLoaded: Boolean get() = lease.isOpen

    /** Wait (off Main) until a previous [unload] really freed the native engine. */
    fun awaitFreed(timeoutMs: Long): Boolean = lease.awaitFreed(timeoutMs)

    /** Router handles only: `update_config` (applies from the next session; models are not reloaded). */
    fun updateRouter(routerConfigJson: String) {
        check(isRouter) { "not a router handle" }
        lease.use(::gone) { h -> if (!guardLink { nativeUpdateRouter(h, routerConfigJson) }) throw lastError("update_config failed") }
    }

    /** Router handles only: close the circuit breaker (Developer "Reset circuit"). */
    fun resetCircuit() { if (isRouter) lease.use({}) { h -> guardLink { nativeResetCircuit(h) } } }

    /** Router handles only: blocking probe of the saved server config; updates the circuit + "Server info". */
    fun probeSaved(): HealthReport {
        val none = { HealthReport(ok = false, error = "error", message = "no router") }
        BlockingCalls.check("nativeProbeRouter")
        return if (!isRouter) none() else lease.use(none) { h -> RouterJson.decodeHealth(guardLink { nativeProbeRouter(h) }) }
    }

    /** D24: `nativeSetLocalLoader` with the model configs as a JSON array; false on an old library. */
    override fun setLocalLoader(configs: List<ModelConfig>): Boolean = isRouter && lease.use({ false }) { h ->
        val json = org.json.JSONArray().apply { configs.forEach { put(org.json.JSONObject(CoreJson.encode(it))) } }.toString()
        try {
            nativeSetLocalLoader(h, json)
        } catch (e: UnsatisfiedLinkError) {
            false
        }
    }

    /** D24 pre-warm (blocking up to the connect timeout). */
    override fun prewarm(): Boolean = isRouter && lease.use({ false }) { h ->
        BlockingCalls.check("nativePrewarm")
        try { nativePrewarm(h) } catch (e: UnsatisfiedLinkError) { false }
    }

    override fun lastRouteDiag(): RouteDiag? = if (!isRouter) null else routeStatus().lastDiag

    /** Router handles only: last route + circuit + last probe. */
    fun routeStatus(): RouteStatus =
        if (!isRouter) RouteStatus() else lease.use({ RouteStatus() }) { h -> RouterJson.decodeRouteStatus(guardLink { nativeRouteStatus(h) }) }

    // ---- D18 cloud STT (router handles; JNI_CONTRACT.md "Cloud STT") ----

    /** Hand the router the API key (D28: id `openrouter`; `null` / "" forgets it). Never logged. */
    fun setCloudKey(providerId: String, key: String?) {
        if (!isRouter) return
        lease.use({}) { h -> if (!guardLink { nativeSetCloudKey(h, providerId, key) }) throw lastError("set_cloud_key failed") }
    }

    /** Blocking "Test speech-to-text" with an `OpenRouterConfig` JSON; `key` null = the router's stored key. */
    fun probeCloud(providerJson: String, key: String?): HealthReport =
        if (!isRouter) Companion.probeCloud(providerJson, key)
        else lease.use({ Companion.probeCloud(providerJson, key) }) { h ->
            BlockingCalls.check("nativeProbeCloud")
            cloudCall { RouterJson.decodeHealth(nativeProbeCloud(h, providerJson, key)) }
        }

    /** This month's counters (`CloudUsage` JSON), or null without a router / symbol. */
    fun cloudUsageJson(): String? =
        if (!isRouter) null else lease.use({ null }) { h -> runCatching { nativeCloudUsage(h) }.getOrNull() }

    /** Restore persisted counters (after open). False on bad JSON or an old library. */
    fun setCloudUsage(json: String): Boolean =
        isRouter && lease.use({ false }) { h -> runCatching { nativeSetCloudUsage(h, json) }.getOrDefault(false) }

    fun resetCloudUsage() { if (isRouter) lease.use({}) { h -> runCatching { nativeResetCloudUsage(h) } } }

    // ---- D28 OpenRouter (JNI_CONTRACT.md "OpenRouter"); a library without them reads as "no cloud" ----

    /** `nativeCleanupStatus`; `supported:false` without a router / the symbol. Non-blocking. */
    fun cleanupStatus(): CleanupStatus =
        if (!isRouter) CleanupStatus() else lease.use({ CleanupStatus() }) { h ->
            runCatching { RouterJson.decodeCleanupStatus(nativeCleanupStatus(h)) }.getOrDefault(CleanupStatus())
        }

    /** `nativeSetCleanup` (merge `{allow_cloud_cleanup?, cleanup_order?, cleanup_off?, cleanup?}`); false + kind `bad_config` on refusal. */
    fun setCleanup(json: String): Boolean = isRouter && lease.use({ false }) { h -> runCatching { nativeSetCleanup(h, json) }.getOrDefault(false) }

    /** The one-time notice of a legacy config this router migrated in memory, else null. */
    fun cloudNotice(): String? = if (!isRouter) null else lease.use({ null }) { h -> runCatching { nativeCloudNotice(h) }.getOrNull() }

    fun ackCloudNotice() { if (isRouter) lease.use({}) { h -> runCatching { nativeAckCloudNotice(h) } } }

    /** `nativeSetCloudModel(use, model)`: false + `bad_config` for an invalid id. Null = no symbol. */
    fun setCloudModel(use: String, model: String): Boolean? =
        if (!isRouter) null else lease.use({ null }) { h ->
            try { nativeSetCloudModel(h, use, model) } catch (e: UnsatisfiedLinkError) { null }
        }

    /** Blocking (≤ 10 s): refresh the catalogue (with the stored key's account filter). Status JSON. */
    fun refreshCloudCatalog(force: Boolean): String {
        BlockingCalls.check("nativeRefreshCloudCatalog")
        return if (!isRouter) Companion.refreshCloudCatalog(force)
        else lease.use({ Companion.refreshCloudCatalog(force) }) { h ->
            try { nativeRefreshCloudCatalog(h, force) } catch (e: UnsatisfiedLinkError) { NO_CLOUD }
        }
    }

    /** Blocking "Check key" (`GET /api/v1/key` with the stored key). */
    fun cloudKeyInfo(): CloudKeyInfo {
        BlockingCalls.check("nativeCloudKeyInfo")
        return if (!isRouter) CloudKeyInfo(error = "cloud_not_configured") else lease.use({ CloudKeyInfo(error = "cloud_not_configured") }) { h ->
            try { RouterJson.decodeKeyInfo(nativeCloudKeyInfo(h)) } catch (e: UnsatisfiedLinkError) { CloudKeyInfo(error = "cloud_not_configured") }
        }
    }

    override fun pushAudio(pcm: ShortArray, length: Int): Boolean =
        lease.use({ false }) { h -> nativePushAudio(h, pcm, length) }

    override fun setContext(ctx: FieldContext) = lease.use({}) { h -> nativeSetContext(h, CoreJson.encode(ctx)) }

    override fun partial(): String? = lease.use({ null }) { h -> nativePartial(h) }

    /** `nativeStartSessionInfo`; a library without it falls back to `nativeStartSession`. */
    override fun startSessionInfo(cfg: SessionConfig): StartInfo = lease.use(::gone) { h ->
        BlockingCalls.check("nativeStartSessionInfo")
        val json = CoreJson.encode(cfg)
        val info = try {
            nativeStartSessionInfo(h, json) ?: throw lastError("start_session failed")
        } catch (e: UnsatisfiedLinkError) {
            if (!nativeStartSession(h, json)) throw lastError("start_session failed")
            return@use StartInfo()
        }
        RouterJson.decodeStartInfo(info)
    }

    /** `nativePoll` (partial + endpoint + live route); `nativePartial` on an older library. */
    override fun poll(): CorePoll = lease.use({ CorePoll() }) { h ->
        try {
            RouterJson.decodePoll(nativePoll(h))
        } catch (e: UnsatisfiedLinkError) {
            CorePoll(partial = nativePartial(h))
        }
    }

    override fun commit(): CommitResult =
        lease.use({ CommitResult(CommitStatus.ERROR, error = "unloaded") }) { h ->
            BlockingCalls.check("nativeCommit")
            CoreJson.decodeCommit(nativeCommit(h))
        }

    override fun cancel() = lease.use({}) { h -> nativeCancel(h) }

    /** A library without `nativeCommand` (pre-2026-10-28) reads as "no AI" (never a crash). */
    override fun command(instruction: String, selected: String?): CommandOutcome =
        lease.use({ CommandOutcome.Failed(CommandOutcome.BACKEND, "Speech engine was unloaded, try again in a moment.") }) { h ->
            BlockingCalls.check("nativeCommand")
            try { CommandOutcome.decode(nativeCommand(h, instruction, selected)) } catch (e: UnsatisfiedLinkError) { CommandOutcome.noLlm }
        }

    override fun learnEdit(pasted: String, edited: String): List<LearnCandidate> =
        lease.use({ emptyList() }) { h -> CoreJson.decodeCandidates(nativeLearnEdit(h, pasted, edited)) }

    /** Never blocks: the native free waits for running calls and happens on a background thread. */
    override fun unload() = lease.close()

    companion object : FlowCoreLoader {
        const val LIBRARY = "flow_ffi"

        @Volatile private var state: LibraryState? = null

        /** Idempotent; never throws. */
        @Synchronized
        fun loadLibrary(): LibraryState {
            state?.let { return it }
            val s = try {
                System.loadLibrary(LIBRARY)
                LibraryState.Loaded(nativeVersion())
            } catch (e: UnsatisfiedLinkError) {
                LibraryState.Unavailable("Speech engine library lib$LIBRARY.so is missing or incompatible: ${e.message}")
            } catch (e: SecurityException) {
                LibraryState.Unavailable("Speech engine library lib$LIBRARY.so could not be loaded: ${e.message}")
            }
            state = s
            return s
        }

        const val LLM_LIBRARY = "flow_llm"
        @Volatile private var llmState: LibraryState? = null

        /**
         * Load the flow-llm plugin into the app's linker namespace so the core's bare-name
         * `dlopen("libflow_llm.so")` finds it (jniLibs are not extracted to disk on API 23+,
         * so there is no file path to hand the core). Idempotent; never throws.
         */
        @Synchronized
        fun loadLlmPlugin(): LibraryState {
            llmState?.let { return it }
            val s = try {
                System.loadLibrary(LLM_LIBRARY)
                LibraryState.Loaded("lib$LLM_LIBRARY.so")
            } catch (e: UnsatisfiedLinkError) {
                LibraryState.Unavailable("AI cleanup plugin lib$LLM_LIBRARY.so is missing from this build (tools/build-native.sh --whisper --llm): ${e.message}")
            } catch (e: SecurityException) {
                LibraryState.Unavailable("AI cleanup plugin lib$LLM_LIBRARY.so could not be loaded: ${e.message}")
            }
            llmState = s
            return s
        }

        override fun loadModels(cfg: ModelConfig): FlowCore {
            val s = loadLibrary()
            if (s is LibraryState.Unavailable) throw CoreException(s.message)
            if (cfg.llmPath != null && cfg.llmLib == null) {
                val l = loadLlmPlugin()
                if (l is LibraryState.Unavailable) throw CoreException(l.message)
            }
            BlockingCalls.check("nativeLoadModels")
            val h = nativeLoadModels(CoreJson.encode(cfg))
            if (h == 0L) throw lastError("load_models failed")
            return NativeFlowCore(h)
        }

        /** Last error + its stable kind (`nativeLastErrorKind`, else recovered from the message prefix). */
        internal fun lastError(fallback: String): CoreException {
            val kind = runCatching { nativeLastErrorKind() }.getOrNull()
            return CoreException.of(nativeLastError(), kind, fallback)
        }

        /** A library built before the router symbols: report, don't crash. */
        private inline fun <T> guardLink(f: () -> T): T = try {
            f()
        } catch (e: UnsatisfiedLinkError) {
            throw CoreException("native library has no engine router (rebuild with tools/build-native.sh --flavor …): ${e.message}", "bad_config")
        }

        /** `nativeOpenRouter`: wraps [local] (ownership moves to the router; may be null) per [routerConfigJson]. */
        val routerOpener = dev.flowclone.data.RouterOpener { json, local ->
            val s = loadLibrary()
            if (s is LibraryState.Unavailable) throw CoreException(s.message)
            val lh = when (local) {
                null -> 0L
                is NativeFlowCore -> local.release()
                else -> throw CoreException("router needs a native local engine, got ${local::class.simpleName}")
            }
            val h = guardLink { nativeOpenRouter(json, lh) }
            if (h == 0L) throw lastError("open_router failed")
            NativeFlowCore(h, isRouter = true)
        }

        /** What the loaded library was built with (`flow_router::Capabilities`); null without the library/symbol. */
        fun capabilities(): Capabilities? {
            if (loadLibrary() !is LibraryState.Loaded) return null
            return runCatching { RouterJson.decodeCapabilities(nativeCapabilities()) }.getOrNull()
        }

        /** Blocking (connect + hello + 3 pings): call off the main thread. Never throws. */
        fun probeServer(serverConfigJson: String): HealthReport {
            BlockingCalls.check("nativeProbeServer")
            val s = loadLibrary()
            if (s is LibraryState.Unavailable) return HealthReport(ok = false, error = "error", message = s.message)
            return try {
                RouterJson.decodeHealth(nativeProbeServer(serverConfigJson))
            } catch (e: UnsatisfiedLinkError) {
                HealthReport(ok = false, error = "error", message = "native library has no engine router: ${e.message}")
            }
        }

        /** A library without the D18 symbols reads as "no cloud" (never a crash). */
        private inline fun cloudCall(f: () -> HealthReport): HealthReport = try {
            f()
        } catch (e: UnsatisfiedLinkError) {
            HealthReport(ok = false, engine = "cloud", error = "cloud_not_configured", message = "native library has no cloud support: ${e.message}")
        }

        /** `nativeCloudCapabilities`; no cloud without the library or the symbol. */
        fun cloudCapabilities(): CloudCapabilities {
            if (loadLibrary() !is LibraryState.Loaded) return CloudCapabilities()
            return runCatching { RouterJson.decodeCloudCapabilities(nativeCloudCapabilities()) }.getOrDefault(CloudCapabilities())
        }

        /** `nativeCloudPresets` (provider templates JSON array), or null. */
        fun cloudPresetsJson(): String? {
            if (loadLibrary() !is LibraryState.Loaded) return null
            return runCatching { nativeCloudPresets() }.getOrNull()
        }

        private const val NO_CLOUD = """{"error":"cloud_not_configured"}"""

        /** D28 one-time migration (`nativeMigrateCloud`, pure, no handle); null without the library / symbol or on bad JSON. */
        fun migrateCloud(devJson: String): String? {
            if (loadLibrary() !is LibraryState.Loaded) return null
            return try { nativeMigrateCloud(devJson) } catch (e: UnsatisfiedLinkError) { null }
        }

        @Volatile private var cacheDirSet: String? = null

        /** Where the catalogue is cached (`<dir>/openrouter-catalog.json`); once per process. No network. */
        @Synchronized
        fun setCloudCacheDir(dir: String) {
            if (cacheDirSet == dir || loadLibrary() !is LibraryState.Loaded) return
            try { nativeSetCloudCacheDir(dir); cacheDirSet = dir } catch (_: UnsatisfiedLinkError) { }
        }

        /** Picker rows JSON for `use` (`stt` / `cleanup`): memory, cache or the baked list. Never the network. */
        fun cloudCatalog(use: String, query: String, filtersJson: String): String {
            if (loadLibrary() !is LibraryState.Loaded) return "[]"
            return try { nativeCloudCatalog(0, use, query, filtersJson) } catch (e: UnsatisfiedLinkError) { "[]" }
        }

        /** Blocking refresh without a router (no account filter). */
        fun refreshCloudCatalog(force: Boolean): String {
            BlockingCalls.check("nativeRefreshCloudCatalog")
            if (loadLibrary() !is LibraryState.Loaded) return NO_CLOUD
            return try { nativeRefreshCloudCatalog(0, force) } catch (e: UnsatisfiedLinkError) { NO_CLOUD }
        }

        /** Test provider before a router exists (temporary native router: [key] is required). Blocking. */
        fun probeCloud(providerJson: String, key: String?): HealthReport {
            BlockingCalls.check("nativeProbeCloud")
            val s = loadLibrary()
            if (s is LibraryState.Unavailable) return HealthReport(ok = false, engine = "cloud", error = "error", message = s.message)
            return cloudCall { RouterJson.decodeHealth(nativeProbeCloud(0, providerJson, key)) }
        }

        // ---- JNI surface (static natives on dev.flowclone.core.NativeFlowCore) ----
        @JvmStatic external fun nativeVersion(): String
        @JvmStatic external fun nativeLastError(): String?
        @JvmStatic external fun nativeLoadModels(modelConfigJson: String): Long
        @JvmStatic external fun nativeUnload(handle: Long)
        @JvmStatic external fun nativeStartSession(handle: Long, sessionConfigJson: String): Boolean
        @JvmStatic external fun nativePushAudio(handle: Long, pcm: ShortArray, length: Int): Boolean
        @JvmStatic external fun nativeSetContext(handle: Long, fieldContextJson: String)
        @JvmStatic external fun nativePartial(handle: Long): String?
        @JvmStatic external fun nativeCommit(handle: Long): String
        @JvmStatic external fun nativeCancel(handle: Long)
        @JvmStatic external fun nativeLearnEdit(handle: Long, pasted: String, edited: String): String

        // ---- 2026-10-22 live route + hands-free endpointing (JNI_CONTRACT.md); every flavour exports them ----
        @JvmStatic external fun nativeStartSessionInfo(handle: Long, sessionConfigJson: String): String?
        @JvmStatic external fun nativePoll(handle: Long): String

        // ---- D17 router additions (JNI_CONTRACT.md "Router"); every symbol above works on router handles too ----
        @JvmStatic external fun nativeLastErrorKind(): String?
        @JvmStatic external fun nativeCapabilities(): String
        @JvmStatic external fun nativeOpenRouter(routerConfigJson: String, localHandle: Long): Long
        @JvmStatic external fun nativeUpdateRouter(handle: Long, routerConfigJson: String): Boolean
        @JvmStatic external fun nativeProbeServer(serverConfigJson: String): String
        @JvmStatic external fun nativeRouteStatus(handle: Long): String
        @JvmStatic external fun nativeResetCircuit(handle: Long)
        @JvmStatic external fun nativeProbeRouter(handle: Long): String

        // ---- D24 never-fail fallback (JNI_CONTRACT.md "On-demand local engine") ----
        @JvmStatic external fun nativeSetLocalLoader(handle: Long, modelConfigsJson: String): Boolean
        @JvmStatic external fun nativePrewarm(handle: Long): Boolean

        // ---- D18 cloud STT (JNI_CONTRACT.md "Cloud STT"); present in every flavour's library ----
        @JvmStatic external fun nativeSetCloudKey(handle: Long, providerId: String, key: String?): Boolean
        @JvmStatic external fun nativeSetCloudKeyFor(handle: Long, providerJson: String, key: String?): Boolean
        @JvmStatic external fun nativeProbeCloud(handle: Long, providerJson: String, key: String?): String
        @JvmStatic external fun nativeCloudUsage(handle: Long): String
        @JvmStatic external fun nativeSetCloudUsage(handle: Long, usageJson: String): Boolean
        @JvmStatic external fun nativeResetCloudUsage(handle: Long)
        @JvmStatic external fun nativeCloudCapabilities(): String
        @JvmStatic external fun nativeCloudPresets(): String

        // ---- D28 OpenRouter (JNI_CONTRACT.md "OpenRouter"); present in every flavour's library ----
        @JvmStatic external fun nativeMigrateCloud(devJson: String): String?
        @JvmStatic external fun nativeSetCleanup(handle: Long, cleanupJson: String): Boolean
        @JvmStatic external fun nativeCleanupStatus(handle: Long): String
        @JvmStatic external fun nativeCloudNotice(handle: Long): String?
        @JvmStatic external fun nativeAckCloudNotice(handle: Long)
        @JvmStatic external fun nativeSetCloudModel(handle: Long, use: String, model: String): Boolean
        @JvmStatic external fun nativeSetCloudCacheDir(dir: String)
        @JvmStatic external fun nativeCloudCatalog(handle: Long, use: String, query: String, filtersJson: String): String
        @JvmStatic external fun nativeRefreshCloudCatalog(handle: Long, force: Boolean): String
        @JvmStatic external fun nativeCloudKeyInfo(handle: Long): String

        // ---- 2026-10-28 Command Mode (JNI_CONTRACT.md "Command Mode"); never null ----
        @JvmStatic external fun nativeCommand(handle: Long, instruction: String, selected: String?): String
    }
}
