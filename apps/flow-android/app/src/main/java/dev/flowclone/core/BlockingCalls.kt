package dev.flowclone.core

/**
 * StrictMode-style guard for review C2: native calls that can block (model load, session start
 * on the server route, commit, probes) must never run on the main thread. [NativeFlowCore] and
 * [FakeFlowCore] call [check] on entry; the app wires [isMainThread] to the main Looper and
 * [onViolation] to a loud log line, tests wire both to assertions.
 */
object BlockingCalls {
    @Volatile var isMainThread: () -> Boolean = { false }
    @Volatile var onViolation: (String) -> Unit = {}

    fun check(what: String) {
        if (isMainThread()) onViolation(what)
    }
}
