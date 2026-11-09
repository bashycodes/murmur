package dev.murmur.command

/**
 * One Command Mode run for History: what was said, what it acted on and what came back.
 * [original] = the selected text (null = speak-to-generate); [result] = the AI's answer (null =
 * none); [outcome] = the debug log's `InsertOutcome` value; [error] = why there is no result.
 */
data class CommandRecord(
    val instruction: String,
    val original: String?,
    val result: String?,
    val appId: String?,
    val outcome: String,
    val error: String? = null,
)
