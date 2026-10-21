package dev.flowclone.core

/**
 * JNI binding to crates/flow-ffi (`libflow_ffi.so`). Exact symbol names and signatures:
 * apps/flow-android/JNI_CONTRACT.md — keep the two in sync.
 *
 * The library is optional at build time: [loadLibrary] reports a missing/broken .so as
 * [LibraryState.Unavailable] instead of throwing, and the UI shows that message.
 */
class NativeFlowCore private constructor(private var handle: Long, val isRouter: Boolean = false) : FlowCore {
    sealed interface LibraryState {
        data class Loaded(val version: String) : LibraryState
        data class Unavailable(val message: String) : LibraryState
    }

    private fun h(): Long = handle.also { check(it != 0L) { "NativeFlowCore used after unload" } }

    override fun startSession(cfg: SessionConfig) {
        if (!nativeStartSession(h(), CoreJson.encode(cfg))) throw lastError("start_session failed")
    }

    /**
     * Gives the handle away (to `nativeOpenRouter`, which takes ownership). This instance is
     * unusable afterwards and its [unload] is a no-op.
     */
    @Synchronized
    fun release(): Long { val x = handle; handle = 0; return x }

    /** Router handles only: `update_config` (applies from the next session; models are not reloaded). */
    fun updateRouter(routerConfigJson: String) {
        check(isRouter) { "not a router handle" }
        if (!guardLink { nativeUpdateRouter(h(), routerConfigJson) }) throw lastError("update_config failed")
    }

    /** Router handles only: last route + circuit + last probe. */
    fun routeStatus(): RouteStatus =
        if (!isRouter || handle == 0L) RouteStatus() else RouterJson.decodeRouteStatus(guardLink { nativeRouteStatus(handle) })

    override fun pushAudio(pcm: ShortArray, length: Int): Boolean =
        handle != 0L && nativePushAudio(handle, pcm, length)

    override fun setContext(ctx: FieldContext) = nativeSetContext(h(), CoreJson.encode(ctx))

    override fun partial(): String? = if (handle == 0L) null else nativePartial(handle)

    override fun commit(): CommitResult =
        if (handle == 0L) CommitResult(CommitStatus.ERROR, error = "unloaded") else CoreJson.decodeCommit(nativeCommit(handle))

    override fun cancel() { if (handle != 0L) nativeCancel(handle) }

    override fun learnEdit(pasted: String, edited: String): List<LearnCandidate> =
        CoreJson.decodeCandidates(nativeLearnEdit(h(), pasted, edited))

    @Synchronized
    override fun unload() {
        val x = handle
        handle = 0
        if (x != 0L) nativeUnload(x)
    }

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
            val s = loadLibrary()
            if (s is LibraryState.Unavailable) return HealthReport(ok = false, error = "error", message = s.message)
            return try {
                RouterJson.decodeHealth(nativeProbeServer(serverConfigJson))
            } catch (e: UnsatisfiedLinkError) {
                HealthReport(ok = false, error = "error", message = "native library has no engine router: ${e.message}")
            }
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

        // ---- D17 router additions (JNI_CONTRACT.md "Router"); every symbol above works on router handles too ----
        @JvmStatic external fun nativeLastErrorKind(): String?
        @JvmStatic external fun nativeCapabilities(): String
        @JvmStatic external fun nativeOpenRouter(routerConfigJson: String, localHandle: Long): Long
        @JvmStatic external fun nativeUpdateRouter(handle: Long, routerConfigJson: String): Boolean
        @JvmStatic external fun nativeProbeServer(serverConfigJson: String): String
        @JvmStatic external fun nativeRouteStatus(handle: Long): String
    }
}
