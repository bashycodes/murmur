package dev.flowclone.insert

/** Outcome of one V3 insertion (Wispr `InsertTextFailureReason` / `VerifySource` enums, 05 §Enums). */
data class InsertResult(
    val succeeded: Boolean,
    val status: Status,
    val reason: FailureReason? = null,
    val source: VerifySource = VerifySource.NONE,
    /** Step names in order (Wispr's trace labels) — diagnostics + tests. */
    val steps: List<String> = emptyList(),
) {
    enum class Status {
        SUCCESS_TEXT_FOUND,
        SUCCESS_TEXT_FOUND_VIA_FALLBACK,
        SUCCESS_NULL_SURROUNDING_TRUSTED_COMMIT,
        SUCCESS_VIA_ACTION_SET_TEXT,
        FAILURE,
        INCONCLUSIVE_FAILURE,
    }

    enum class FailureReason {
        INPUT_NOT_STARTED,
        INPUT_CONNECTION_NULL,
        SESSION_MISMATCH,
        COMMIT_TEXT_THREW,
        COMMIT_TEXT_SILENT_FAILURE,
        INPUT_SESSION_ENDED,
        VERIFICATION_INCONCLUSIVE,
        INCONCLUSIVE_NULL_SURROUNDING,
        EXCLUDED_FIELD,
        DEADLINE_EXHAUSTED,
    }

    enum class VerifySource { SURROUNDING_TEXT, NODE_TEXT, TRUSTED, NONE }
}
