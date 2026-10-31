# Murmur

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

## Server access

**Key**:
A named secret that lets one person or device use the self-hosted server; pasted into the app's Token field. The server keeps only its hash, a stable id and usage counts. Revoking it locks out that device alone.
_Avoid_: token (except for the app's field name), password, account, user

**Admin key**:
A key with the admin flag, for managing the server remotely (the admin tools come later). Plain keys only dictate.
_Avoid_: root key, master token

**Owner**:
The admin key named `owner` that the old single server token became on upgrade, so existing installs keep working unchanged.
_Avoid_: default token, legacy key
