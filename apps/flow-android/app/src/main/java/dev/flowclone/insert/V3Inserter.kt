package dev.flowclone.insert

import dev.flowclone.insert.InsertResult.FailureReason
import dev.flowclone.insert.InsertResult.Status
import dev.flowclone.insert.InsertResult.VerifySource
import dev.flowclone.policy.BubbleVisibilityPolicy
import dev.flowclone.policy.EditorInputType
import dev.flowclone.policy.PolicyLists
import kotlinx.coroutines.delay

/**
 * Wispr `KeepAliveFirstTextInserter.insert` (V3, smali-verified; spec 05 §Flow, 07 §3.3),
 * with every V3 feature on (`v3_attack_miss_hint_aware`).
 *
 * Contract: `commitText` at most once; verify by polling for 700 ms; fall back to
 * `ACTION_SET_TEXT` only into an empty / placeholder field; whole run bounded by
 * 3500 ms from [insert]'s `t0` (taken before the caller's mutex, so queueing eats budget).
 * The caller owns the terminal clipboard fallback ([InsertionCoordinator]).
 */
class V3Inserter(private val env: InsertionEnvironment, private val now: () -> Long) {
    companion object {
        const val DEADLINE_MS = 3500L
        const val SESSION_WAIT_MS = 500L
        const val VERIFY_WINDOW_MS = 700L
        const val MIN_VERIFY_BUDGET_MS = 300L
        const val SETTLE_MS = 100L
        const val STALE_PROBE_RESERVE_MS = 400L
        const val MAX_NODE_TIER_FIELD_LENGTH = 100_000
        const val PROBE_CONTEXT_CHARS = 500
    }

    private sealed interface Probe {
        data class Available(val text: String, val source: VerifySource) : Probe
        data object Unavailable : Probe
    }

    private inner class Run(val text: String, t0: Long) {
        val deadline = t0 + DEADLINE_MS
        val steps = mutableListOf<String>()
        var staleProbed = false
        var precommit: String? = null

        fun remaining() = deadline - now()
        fun step(s: String) { steps += s }
        fun ok(status: Status, source: VerifySource) = InsertResult(true, status, null, source, steps.toList())
        fun fail(reason: FailureReason?, status: Status = Status.FAILURE) =
            InsertResult(false, status, reason, VerifySource.NONE, steps.toList())

        fun anchorPkg() = env.dictationStartPackage() ?: env.currentPackage()
        fun isOurPackage(p: String?) = p != null && (p == env.dictationStartPackage() || p == env.currentPackage())
        fun isExcludedInputType() = env.editorInputType()?.let(EditorInputType::isSensitive) ?: false
        fun connection() = if (env.isInputSessionActive()) env.currentInputConnection() else null

        fun livenessReadExcluded(): Boolean {
            val t = env.editorInputType() ?: return true
            return EditorInputType.isContentExtractionExcluded(t) || BubbleVisibilityPolicy.isBanking(anchorPkg())
        }

        fun reacquireEnabledForSurface(): Boolean {
            val p = anchorPkg() ?: return false
            return p in PolicyLists.IN_CALL || p in PolicyLists.SYSTEM_SEARCH || p in PolicyLists.KNOWN_NULL_SURROUNDING_TEXT
        }

        fun boundEditorMatchesAnchor() = env.imeBoundPackage().let { it != null && it == anchorPkg() }

        fun staleProbeEnabledForSurface() =
            boundEditorMatchesAnchor() && anchorPkg() in PolicyLists.RESTART_PRONE_READABLE && !livenessReadExcluded()

        fun probeConnectionAlive(ic: InputConnectionPort) =
            !livenessReadExcluded() && runCatching { ic.getSurroundingText(0, 0) }.getOrNull() != null

        fun sessionReason(): FailureReason = when {
            env.isInputSessionActive() -> FailureReason.INPUT_CONNECTION_NULL
            env.hasSessionEverStarted() -> FailureReason.INPUT_SESSION_ENDED
            else -> FailureReason.INPUT_NOT_STARTED
        }

        fun isExcludedNode(n: EditableNode) = n.isPassword || EditorInputType.isContentExtractionExcluded(n.inputType)

        fun knownHints() = TextMatching.knownComposeHints(env.currentPackage() ?: anchorPkg(), staleProbed)

        fun readNode(n: EditableNode?): String? {
            if (n == null || isExcludedNode(n)) return null
            return TextMatching.readNodeText(n.snapshot(), env.editorHint(), knownHints())
        }

        /** SurroundingTextProbe.read(ic, node, 500, 500). */
        fun probeRead(ic: InputConnectionPort, node: EditableNode?): Probe {
            val st = runCatching { ic.getSurroundingText(PROBE_CONTEXT_CHARS, PROBE_CONTEXT_CHARS) }.getOrNull()
            st?.text?.let { return Probe.Available(it, VerifySource.SURROUNDING_TEXT) }
            readNode(node)?.let { return Probe.Available(it, VerifySource.NODE_TEXT) }
            return Probe.Unavailable
        }

        suspend fun nodeTier(reason: FailureReason, passed: EditableNode? = null): InsertResult {
            step("node_tier_started")
            if (now() >= deadline) return fail(FailureReason.DEADLINE_EXHAUSTED).also { step("deadline_exhausted") }
            val node = passed ?: env.findFocusedInputNode()
                ?: env.relocateDictationStartNode()?.also { step("node_relocated") }
                ?: return fail(reason).also { step("node_tier_no_node") }
            if (!isOurPackage(node.packageName)) return fail(reason).also { step("node_tier_wrong_package") }
            if (!node.isEditable) return fail(reason).also { step("node_tier_not_editable") }
            if (isExcludedNode(node)) return fail(FailureReason.EXCLUDED_FIELD).also { step("node_tier_excluded_field") }
            if (text.length > MAX_NODE_TIER_FIELD_LENGTH) return fail(reason).also { step("node_tier_field_too_large") }
            if (now() >= deadline) return fail(FailureReason.DEADLINE_EXHAUSTED)
            if (!node.refresh()) return fail(reason).also { step("node_tier_refresh_failed") }
            return when (TextMatching.compose(text, readNode(node))) {
                TextMatching.Compose.AlreadyPresent -> {
                    step("node_tier_already_present")
                    ok(Status.SUCCESS_TEXT_FOUND_VIA_FALLBACK, VerifySource.NODE_TEXT)
                }
                TextMatching.Compose.RefusedUnreadable, TextMatching.Compose.RefusedNonEmpty ->
                    fail(reason).also { step("node_tier_refused") }
                TextMatching.Compose.Write -> {
                    if (!node.canSetText || !runCatching { node.setText(text) }.getOrDefault(false))
                        return fail(reason).also { step("node_tier_failed") }
                    repeat(2) {
                        delay(SETTLE_MS)
                        node.refresh()
                        if (node.snapshot().text?.trim() == text.trim()) {
                            step("node_tier_written")
                            return ok(Status.SUCCESS_VIA_ACTION_SET_TEXT, VerifySource.NODE_TEXT)
                        }
                    }
                    fail(reason).also { step("node_tier_failed") }
                }
            }
        }

        suspend fun execute(): InsertResult {
            if (text.isEmpty()) return ok(Status.SUCCESS_TEXT_FOUND, VerifySource.NONE).also { step("empty_text") }
            if (now() >= deadline) return fail(FailureReason.DEADLINE_EXHAUSTED, Status.INCONCLUSIVE_FAILURE)
            if (isExcludedInputType()) return fail(FailureReason.EXCLUDED_FIELD).also { step("excluded_input_type") }
            if (env.isInNotificationReplyContext()) {
                step("reply_node_path")
                return nodeTier(sessionReason())
            }

            var ic = connection()
            if (ic == null) {
                val wait = minOf(SESSION_WAIT_MS, remaining())
                if (wait > 0 && reacquireEnabledForSurface()) {
                    step("reacquire_attempt")
                    step(if (env.reacquireInputSession()) "reacquire_nudged" else "reacquire_failed")
                }
                if (wait > 0) {
                    step("session_wait")
                    step(if (env.awaitInputStart(wait)) "session_wait_resolved" else "session_wait_timeout")
                }
                ic = connection() ?: return nodeTier(sessionReason())
            }

            run {
                val bound = env.imeBoundPackage()
                val focused = env.focusedInputPackage()
                if (bound != null && focused != null && bound != focused) {
                    step("session_mismatch")
                    val wait = minOf(SESSION_WAIT_MS, remaining())
                    if (isOurPackage(focused) && wait > 0) {
                        step("reacquire_attempt")
                        env.reacquireInputSession()
                        env.awaitInputStart(wait)
                    }
                    ic = connection() ?: return nodeTier(FailureReason.INPUT_CONNECTION_NULL)
                    val b2 = env.imeBoundPackage()
                    val f2 = env.focusedInputPackage()
                    if (b2 != null && f2 != null && b2 != f2) return nodeTier(FailureReason.SESSION_MISMATCH)
                    step("session_mismatch_resolved")
                }
            }

            if (staleProbeEnabledForSurface() && now() < deadline) {
                staleProbed = true
                if (env.boundEditorRestarted()) step("editor_restarted")
                if (probeConnectionAlive(ic!!)) {
                    step("precommit_probe_ok"); precommit = "ok"
                } else {
                    step("precommit_probe_null"); precommit = "null"
                    val wait = minOf(SESSION_WAIT_MS, remaining() - STALE_PROBE_RESERVE_MS)
                    if (wait > 0 && isOurPackage(env.focusedInputPackage())) {
                        step("stale_reacquire_attempt")
                        env.reacquireStaleSession()
                        if (env.awaitInputStart(wait)) step("stale_reacquire_restarted")
                        ic = connection() ?: return nodeTier(sessionReason()).also { step("stale_reacquire_failed") }
                        if (probeConnectionAlive(ic!!)) step("stale_reacquire_resolved")
                        val b = env.imeBoundPackage()
                        val f = env.focusedInputPackage()
                        if (b != null && f != null && b != f && !isOurPackage(f)) return nodeTier(FailureReason.SESSION_MISMATCH)
                    }
                }
            }

            if (isExcludedInputType()) return fail(FailureReason.EXCLUDED_FIELD).also { step("excluded_input_type") }
            if (remaining() < MIN_VERIFY_BUDGET_MS)
                return fail(FailureReason.DEADLINE_EXHAUSTED, Status.INCONCLUSIVE_FAILURE).also { step("insufficient_verify_budget") }

            step("commit")
            val conn = ic!!
            try {
                conn.commitText(text)
            } catch (e: Exception) {
                step("commit_threw")
                return nodeTier(FailureReason.COMMIT_TEXT_THREW)
            }

            val verifyDeadline = minOf(deadline, now() + VERIFY_WINDOW_MS)
            val node = env.findFocusedInputNode()
            var reads = 0
            var prev: String? = null
            var anyReadable = false
            var stable = false
            while (true) {
                val settle = minOf(SETTLE_MS, verifyDeadline - now())
                if (settle > 0) delay(settle)
                if (now() >= verifyDeadline) break
                val p = probeRead(conn, node)
                reads++
                if (p !is Probe.Available) {
                    if (reads == 1 && precommit != "null" && env.currentPackage() in PolicyLists.KNOWN_NULL_SURROUNDING_TEXT) {
                        step("trusted_commit")
                        return ok(Status.SUCCESS_NULL_SURROUNDING_TRUSTED_COMMIT, VerifySource.TRUSTED)
                    }
                    continue
                }
                anyReadable = true
                if (TextMatching.containsCommittedText(p.text, text)) {
                    step("read_text_found")
                    return if (p.source == VerifySource.SURROUNDING_TEXT) ok(Status.SUCCESS_TEXT_FOUND, p.source)
                    else ok(Status.SUCCESS_TEXT_FOUND_VIA_FALLBACK, p.source)
                }
                stable = prev != null && prev == p.text
                prev = p.text
            }
            return when {
                reads == 0 -> fail(FailureReason.VERIFICATION_INCONCLUSIVE, Status.INCONCLUSIVE_FAILURE).also { step("read_all_null") }
                !anyReadable -> { step("read_all_null"); nodeTier(FailureReason.INCONCLUSIVE_NULL_SURROUNDING, node) }
                stable -> { step("read_confirmed_miss"); nodeTier(FailureReason.COMMIT_TEXT_SILENT_FAILURE, node) }
                else -> { step("read_inconclusive"); nodeTier(FailureReason.VERIFICATION_INCONCLUSIVE, node) }
            }
        }
    }

    suspend fun insert(text: String, t0: Long): InsertResult = Run(text, t0).execute()
}
