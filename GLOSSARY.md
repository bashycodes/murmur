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

**Route order**:
The ordered list of speech routes (own server, this device, cloud) a dictation tries; when one fails, the next takes over. A meeting's chunks follow the same order.
_Avoid_: fallback chain, prefer (the old two-route setting)

**Meeting**:
A recording of any length (a meeting, a phone call) transcribed into timestamped segments, rather than typed into a text field. It is cut at pauses into chunks of at most 30 s, and each chunk is transcribed as one ordinary dictation.
_Avoid_: long dictation, notetaker session (Notetaker is the Windows feature that records one)

**Call transcript**:
The Android Calls page's result: a meeting transcript of one call recording the user picked, made on the phone by its own engine and kept in the app. The recording itself is only read.
_Avoid_: call log, call recording (the audio file, which is not ours)

**Segment**:
One timestamped piece of a meeting transcript: start, end (ms from the start of the recording) and text.
_Avoid_: chunk (the audio behind it), paste segment, utterance

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
