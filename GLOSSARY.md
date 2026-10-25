# Flow Clone

A dictation app: the user speaks, and polished text is inserted where they are typing.

## Language

**Transcript**:
The words the speech model heard, before any cleanup.
_Avoid_: raw text, ASR output

**AI cleanup**:
The language-model step that turns a transcript into finished text: punctuation, sentence and paragraph breaks, spoken commands such as "quote unquote" or "in parentheses", quotation marks, hyphens, numbers, self-corrections, lists, tone and light edits (dropping fillers, short trims, unit abbreviations), all in one pass.
_Avoid_: post-processing, formatting rules, LLM cleanup, spoken commands (as a separate stage)

**Fallback rules**:
The fixed text rules used only when AI cleanup cannot run or its answer is rejected.
_Avoid_: deterministic layer, spoken cleanup
