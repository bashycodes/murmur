# Murmur

A dictation app: the user speaks, and polished text is inserted where they are typing.

## Language

**Transcript**:
The words the speech model heard, before any cleanup.
_Avoid_: raw text, ASR output

**AI cleanup**:
The language-model step that turns a transcript into finished text: punctuation, sentence and paragraph breaks, spoken commands such as "quote unquote" or "in parentheses", quotation marks, hyphens, numbers, self-corrections, lists, tone and light edits (dropping fillers, short trims, unit abbreviations), all in one pass.
_Avoid_: post-processing, formatting rules, LLM cleanup, spoken commands (as a separate stage)

**Keyboard language**:
The language setting (Android default, Windows "Keyboard language" on) that makes a dictation use the language of the keyboard active when the recording stops (D58): a keyboard switched mid-recording changes the language, and the whole recording is transcribed again in it. The start keyboard only opens the live session (D37).
_Avoid_: input language, IME language, start language (as the result's language)

**Cloud**:
Online models the app can use, through one **Cloud provider** at a time (enforced by each app's settings layer): OpenRouter (speech to text and AI cleanup) or the user's LiteLLM proxy (AI cleanup only for now). The UI says "Cloud"; a provider's name appears only in the provider switch and its own fields, and in consent refusals, which name the destination (provider and host, e.g. "LiteLLM (10.0.0.5)") because consent is bound to it (D52, D78). Other messages say "the cloud".
_Avoid_: OpenRouter (as the umbrella term), online service

**LiteLLM proxy**:
The Cloud provider that is the user's own OpenAI-compatible server (typically LiteLLM; vLLM or Ollama work too). In code and settings it is still `custom` (`cloud.cleanup_provider = "custom"`, `cloud.custom_cleanup`, key id `custom`; D52, D78).
_Avoid_: custom server (in UI copy), local server (that is murmur-server), your server (that is murmur-server)

**Fallback rules**:
The fixed text rules used only when AI cleanup cannot run or its answer is rejected.
_Avoid_: deterministic layer, spoken cleanup

**Route order**:
The ordered list of speech routes (own server, this device, cloud) a dictation tries; when one fails, the next takes over. A meeting's chunks follow the meeting order.
_Avoid_: fallback chain, prefer (the old two-route setting)

**Meeting order**:
On Windows, the route order meeting transcription uses (Settings › Speech to text › Meetings), with its own on/off switches; the same as the dictation route order until the user changes it.
_Avoid_: meeting routes, meeting fallback chain

**Meeting**:
A recording of any length (a meeting, a phone call) transcribed into timestamped segments, rather than typed into a text field. It is cut at pauses into chunks of at most 30 s, and each chunk is transcribed as one ordinary dictation.
_Avoid_: long dictation, notetaker session (Notetaker is the Windows feature that records one)

**Call transcript**:
The Android Calls page's result: a meeting transcript of one call recording the user picked, made on the phone by its own engine and kept in the app. The recording itself is only read.
_Avoid_: call log, call recording (the audio file, which is not ours)
**Notetaker**:
The Windows feature that records a meeting (microphone + what the PC plays) and turns it into a transcript of Me / Others turns on the Meetings page.
_Avoid_: meeting bot, call recorder

**Meeting summary**:
A short AI summary plus action items of one meeting transcript, written by the AI cleanup engines in the meeting's language and kept with the meeting; Regenerate writes it again.
_Avoid_: minutes, recap, notes (the per-chunk notes a long meeting is first condensed into)

**Meeting detection**:
The Notetaker's watch for a Zoom, Webex or Jabber call (a meeting window or the app's microphone stream, not the app merely running) that asks to record it or records it, and stops that recording when the call ends.
_Avoid_: auto-record (one of its two modes), call detection on Android (no such feature)

**Me / Others**:
The speaker label of a meeting turn, by audio source: Me = the microphone, Others = what the PC plays (the other side of the call). Not voice recognition.
_Avoid_: speaker 1/2, diarization

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
