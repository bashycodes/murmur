# Murmur features you might miss

Murmur works without any setup: hold a key (Windows) or tap the bubble
(Android), speak, and the cleaned-up text is typed where your cursor is.
Many of its features stay out of sight until you need them. This page lists
the ones you probably won't find by yourself. It is not a full manual.

Each entry says which app has it: **Windows**, **Android** or **Both**.
Settings paths use the labels you see in the app. Shortcuts are the
defaults; you can change them all in Windows under *Settings → General →
Shortcuts*.

## Hotkeys and gestures

**Double-tap to go hands-free** (Windows). Tap the push-to-talk key twice
within half a second to keep listening without holding anything. The
dictation stops when you go quiet, or when you click ✓ (finish) or ✕
(cancel) on the Murmur bar.
*How:* double-tap **Right Alt**. Its own shortcut is **Ctrl+Win+Space**.

**Paste or copy your last dictation** (Windows). The text you dictated last
is typed in again, or put on the clipboard. This helps when it went into
the wrong window.
*How:* **Shift+Alt+Z** pastes it, **Shift+Alt+X** copies it. (There is no
tray item for this.)

**Cancel a dictation** (Both). Throws away what you are saying, and nothing
is typed.
*How:* Windows: **Esc**. Android: ✕ next to the bubble.

**Push to talk steps aside for Windows shortcuts** (Windows). If you hold
the push-to-talk keys and then press another key, such as Ctrl+Win+Left to
switch desktops, Murmur takes it as a Windows shortcut and drops the
recording.
*How:* this is automatic.

**Open Settings from the keyboard** (Windows).
*How:* **Ctrl+,** while a Murmur window is focused. It only works there,
so it never takes a key away from another app.

**Hold the bubble to talk** (Android). Tapping the bubble starts a
hands-free dictation. Pressing and holding it records only while you hold,
like a walkie-talkie, and sends when you let go.
*How:* long-press the bubble.

**Snooze the bubble, then shake it back** (Android). Drag the bubble onto
the target at the bottom of the screen to hide it for 10 minutes. Shake
the phone to bring it back sooner, or to show a hidden bubble for a few
seconds.
*How:* drag it down to snooze. Shaking is on by default: *Settings →
Bubble size → Shake to restore*.

**A smaller bubble while you're not using it** (Android). After a few idle
seconds the bubble shrinks, down to a small dot if you like. You can also
change its size.
*How:* *Settings → Bubble size* (Shrink, Shrink to a dot, size 75–150%).

**Launcher shortcuts for notes** (Android). Long-press the Murmur icon to
dictate straight into a **New note** or the **Quick note**.

## Smarter text

**Spoken punctuation and formatting** (Both). You can say "comma",
"period", "question mark", "colon", "new line" and "new paragraph". "Quote
… unquote", "in parentheses", "open bracket … close bracket" and "dash" add
the marks too. Amounts, times and dates become digits ("forty dollars" →
$40, "three thirty pm" → 3:30 pm).
*How:* this is on by default. Android: *Settings → Language →
Auto-format*.

**Change your mind mid-sentence** (Both). Say "scratch that" or "never
mind" and the part before it is dropped. "No wait", "actually I mean" and
"or rather" keep only the correction: "meet at five, no wait, six" becomes
"meet at six". Filler words and stutters ("um", "I I think") are removed.
*How:* this is automatic.

**Say "press enter" to send** (Windows). End a dictation with "press enter"
and Murmur types the text, then presses Enter. In a chat app that sends
the message.
*How:* say it as the last words. "Send it press enter." types "Send it."
and sends it.

**Spoken lists become real lists** (Both). Items you number out loud
("first … second …") become a numbered list. Other lists of three or more
items ("a few things for the trip: tent, stove and maps") become a list
when AI cleanup is on.
*How:* numbered lists come from Auto-format, which is on by default
(English). For other lists, turn on AI cleanup on the device, your server
or the cloud.

**Transforms: Polish and Prompt Builder** (Windows). Select text in any
app, press a shortcut, and the AI rewrites it in place. **Polish** makes
it clearer and shorter. **Prompt Builder** turns a rough request into a
well-structured prompt for an AI assistant. You can add your own
transforms ("Make it friendlier"), and set one to run on every dictation.
*How:* left sidebar **Transforms → Opt in to Transforms** (off by default).
Then **Win+Alt+1** runs Polish, **Win+Alt+2** runs Prompt Builder, and
**Win+Alt+O** shows what the last transform changed.

**Command Mode** (Both). Select text and say what to do with it: "make it
shorter", "turn this into bullet points". The selection is replaced with
the result. Ask a question instead ("what does this mean?") and the answer
shows in a card, and your text is left alone. With nothing selected, what
you say is written new at the cursor (Android: only with Command Mode set
to *Always*).
*How:* Windows: hold **Ctrl+Win+Alt** and speak. Android: tap ✎ next to
the bubble; *Settings → Bubble size → Command Mode* sets when ✎ shows. You need
AI cleanup on the device or in the cloud; your server is not used for
Command Mode. The cloud is used only if you allow it to get the text
around the cursor (Windows: *AI cleanup → Advanced → Also send to the
cloud → Text around the cursor*; Android: *Settings → Cloud → Send text
around the cursor*).

**Snippets** (Both). Say a short phrase and get a long text you saved,
such as "my calendar link" → the link.
*How:* Windows: left sidebar **Snippets**. Android: the **Snippets** tab. On Windows, say the phrase on its own.

**Dictionary and learning from your fixes** (Both). Add names and jargon
so they are heard and spelled your way. With "heard as", a word that is
often misheard is always replaced. If you fix a word right after a
dictation, Murmur suggests adding it. Nothing is added until you say so.
*How:* Windows: sidebar **Dictionary**, and *Settings → General → Learn
from my corrections*. Android: *Dictionary*, and *Settings → Data &
privacy → Learn from my edits*. Both are on by default. You can import and
export the list as JSON.

**A writing style per kind of app** (Both). Chats get a casual tone,
email and everything else a formal one. You can change the style for each
group and move a single app into another group.
*How:* Windows: sidebar **Style** (with *Style per app* below it).
Android: the **Style** tab, or *Settings → Writing style* and *Style per
app*.

**Retry with current settings** (Both). Murmur keeps the audio of your
newest 20 dictations. After you change the model or the cleanup, run one
of them again and compare; earlier versions are kept.
*How:* the Retry button on a dictation in History (Windows: Home).
The number kept is in *Settings → Data and privacy* (Windows) or *Settings
→ Keep history → Keep recordings for retry* (Android). Password fields are never kept.

## Context

**Text around the cursor** (Both). Before cleanup, Murmur reads the field
you are typing in: up to 500 characters before and after the cursor, and
any selection. A dictation in mid-sentence then starts in lower case, gets
the right spacing, and doesn't repeat what is already there. This is only
done on the device, and never in password fields.
*How:* this is on by default. Windows: *Settings → AI cleanup → Advanced →
Use text on screen*.

**App and on-screen context for cloud cleanup**. Cloud cleanup can also
get the app's name, the text around the cursor and the words on screen
(Windows: the window title; Android: the visible text of the app's window,
up to 2000 characters, never password or sensitive fields nor the field you
type in). Each one is off until you turn it on. On Android, the words on
screen, once on, also help speech recognition on the phone and on your
server, as the text around the cursor does; cloud speech-to-text never gets
them, and cloud cleanup only with its consent.
*How:* Windows: *Settings → AI cleanup → Advanced → Also send to the cloud*
(App name, Text around the cursor, Words on screen). Android: *Settings →
Cloud → Send text around the cursor* and *Send on-screen words*.

**Excluded apps** (Android). The bubble stays away from banking and
payment apps; this built-in list is always on. You can add more apps.
*How:* *Settings → Excluded apps*.

## Languages

**Translate as you dictate** (Both). Speak in one language and the text is
typed, cleaned up, in another. By default the other language is your
second keyboard language, so with English and Hebrew keyboards, English
comes out as Hebrew and Hebrew as English.
*How:* Windows: hold **Ctrl+Win+Shift**. The target language is under
*Settings → General → Translate into*. Android: tap ⇄ next to the bubble.
It shows while two or more keyboard languages are on; see *Settings →
Bubble size → Translate*. It works best with your server or the cloud; the
on-device model translates poorly.

**The language follows your keyboard** (Both). Murmur uses the language of
the keyboard that is active when you stop speaking. Switch the keyboard to
Hebrew halfway through and the whole dictation is transcribed as Hebrew.
*How:* this is the default (*Keyboard language* under *Settings → General
→ Languages* on Windows, and *Settings → Language* on Android).

**A separate Hebrew model** (Both). Hebrew dictations go to their own
speech model, and everything else to your main one.
*How:* Windows: *Settings → Speech to text → Advanced*. Android: *Settings
→ Speech to text*. *Keep both languages loaded* (Android, off by default)
removes the wait when you switch, at the cost of about 1 GB of memory.

## Hands-free and live text

**Stop after a pause** (Both). A hands-free dictation can end by itself after
a pause you choose (for example 1.5 seconds of silence). Off by default: a
hands-free dictation runs until you stop it. Holding a key or the bubble is
never cut off.
*How:* Windows: *Settings → General → Hands-free stops after*. Android:
*Settings → Timing and limits → Stop after silence (ms)*. "Never" (Windows) or
0 (Android) turns it off.

**Live transcription** (Both). Murmur transcribes while you speak, so a
long dictation is ready soon after you stop.
*How:* this is on by default. Windows: *Settings → Speech to text → Live
transcription*. Android: *Settings → Speech to text → Transcribe while I
speak*.

**Live preview** (Both). Shows the words next to the Murmur bar or the
bubble as you speak.
*How:* this is off by default. Windows: *Settings → General → Live
preview*. Android: *Settings → Bubble size → Live preview*.

## Notes and recordings

**Scratchpad and notes** (Windows). Notes you dictate and keep on this PC.
One shortcut makes a new note and starts a hands-free dictation into it.
*How:* sidebar **Scratchpad**, or tray **Notes…**. *Settings → General*
has shortcuts for **Open Scratchpad** and **New note and dictate**; neither
has one by default.

**Transcribe call recordings** (Android). Pick an audio file or call
recording and the phone turns it into text. Optionally, new call
recordings are transcribed automatically.
*How:* the **Calls** page in the drawer. (There is no live meeting
notetaker yet. The Windows meeting recorder is hidden in this version.)

## Privacy and self-hosting

**Your own server, with a safety net** (Both). Run `murmur-server` on
your own machine and your PCs and phones dictate through it over your LAN
or VPN. If the server can't be reached, the dictation is done on the
device instead and nothing is lost.
*How:* [server-setup.md](server-setup.md). Then *Settings → Your server*
in each app. To have the on-device model ready at once, use *Keep model
loaded* (Windows: Speech to text → Advanced) or *Keep loaded while the
server is used* (Android, on by default).

**Engine order** (Both). Choose which engines are tried in which order:
this device, your server, the cloud. If one fails, the next one gets the
same audio.
*How:* Android: *Settings → Speech to text* and *AI cleanup*. Windows: *Settings → Speech
to text* and *AI cleanup*.

**One key per person or device** (murmur-server, Windows admin). Give each
device its own key, see when it was last used, and revoke it alone.
*How:* `murmur-server keys add <name>`, or in Windows *Settings → Users &
keys* when the app uses an admin key. See
[server-advanced.md](server-advanced.md) §4.

**Models from your own server** (Both). The apps download speech and
cleanup models from your server's model library, so a network with no
internet works.
*How:* [air-gapped.md](air-gapped.md). Windows: *Settings → Your server →
Models on your server*.

**Cloud only if you ask** (Both). Cloud speech-to-text and cleanup
(OpenRouter, or your own OpenAI-compatible server such as LiteLLM) are
off until you add a key and agree, for each provider.
*How:* Windows: *Settings → AI provider*. Android: *Settings → Cloud*.
Details: [cloud-stt.md](cloud-stt.md).

**Back up and restore** (Android). Notes, History, Dictionary, Insights
and settings in one JSON file, for a new phone or a reinstall. Keys and
models are not included.
*How:* *Settings → Back up & restore → Export all data / Restore from a
backup*. Format: [backup-format.md](backup-format.md).

**Insights stay on the device** (Both). Words dictated, speaking speed,
streaks, time saved and your top apps are counted on the device from word
counts only, with no text.
*How:* sidebar **Insights** (Windows) or **Insights** in the drawer
(Android).

## Power user

**Which engine did that?** (Both). A small **L**, **S** or **C** after
each dictation shows whether this device, your server or the cloud did
it.
*How:* Windows: *Settings → System → Show which engine was used*.
Android: *Developer settings → Engine badge*.

**Paste that works everywhere** (Windows). For apps that ignore Ctrl+V,
switch to Shift+Insert or typing. Your clipboard is put back after each
paste.
*How:* *Settings → System → Paste with* and *Restore clipboard after
pasting*.

**Hidden developer settings** (Android). Tap the version line under
*Settings → About* seven times. You get the server status, a debug log
and the engine badge.

**Run setup again** (Windows). Go through setup again and keep your
current choices; before Finish it tells you what will change.
*How:* *Settings → General → Run setup*.

**Server debug capture** (murmur-server). When a dictation through the
server comes out wrong, the server can keep the audio so you can replay
it. It is off by default.
*How:* [server-advanced.md](server-advanced.md) §9.
