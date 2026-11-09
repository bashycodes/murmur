# Spec: text post-processing and edit learning

Status: the behaviour contract for `crates/flow-core/src/postprocess/`
(minus `spoken*.rs` and `terminal.rs`, which are specified by their own
module docs and tests) and `crates/flow-core/src/learn.rs`. Written
2026-11-09 for a from-scratch rewrite of those modules.

Sources: the public API and its callers (`engine.rs`, `flow-ffi`,
`flow-router`, the Windows and Android shells), `docs/DECISIONS.md`,
`docs/core-architecture.md`, `CHANGELOG.md`, this repo's own tests, and
the observed output of the pre-rewrite code on invented inputs
(`tools/probes/prototypes/cleanroom_2026_11_09/`: `cases.json` from
`gen_cases.py`, outputs in `before.jsonl`). Where this spec departs from
an observed output, the row says so and why (**Change**).

## 1. Where it runs

The engine's commit path is: hallucination guard → spoken cleanup and
layout (`spoken`) → closing period (`terminal`) → AI cleanup (optional) →
**post-processing** (this spec) → paste segments (`paste`). Post-processing
gets the cleaned text, the raw transcript, the session config and the
focused-field context, and returns the text to insert plus whether the
shell should press Enter afterwards.

It is deterministic, has no I/O, and must stay fast (it runs on every
commit; the dictionary matcher is built once per distinct dictionary and
cached by the engine).

## 2. Public API (must keep compiling)

```rust
// crates/flow-core/src/postprocess/mod.rs
pub mod spoken;      // unchanged
pub mod terminal;    // unchanged
pub use DictMatcher; // see §4

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostOutput { pub text: String, pub press_enter: bool, pub showed_prompt: bool }

pub fn apply(text: &str, raw_asr: &str, cfg: &SessionConfig, ctx: &FieldContext) -> PostOutput;
pub fn apply_with(text: &str, raw_asr: &str, cfg: &SessionConfig, ctx: &FieldContext,
                  m: &DictMatcher) -> PostOutput;
pub fn effective_category(platform: Platform, ctx: &FieldContext) -> AppCategory;
pub fn starts_with_proper_noun(text: &str, proper: &[String], dict: &[DictEntry]) -> bool;

impl DictMatcher {
    pub fn new(personal: &[DictEntry], team: &[DictEntry]) -> Self;
    pub fn is_empty(&self) -> bool;
    pub fn whole_snippet(&self, text: &str) -> Option<String>;
    pub fn find_and_replace(&self, text: &str) -> String;
}

// crates/flow-core/src/learn.rs
pub const MAX_RUN_WORDS: usize = 4;
pub const MAX_CANDIDATES: usize = 4;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate { pub original: String, pub corrected: String,
                       #[serde(default)] pub looks_like_name: bool }
pub fn looks_like_name(corrected: &str, sentence_initial: bool) -> bool;
pub fn learn_edit(pasted: &str, edited: &str, dict: &Dictionary) -> Vec<Candidate>;
```

Callers: `engine.rs` (`apply_with`, `effective_category`,
`DictMatcher::{new, whole_snippet}`, `spoken::*`, `terminal::close_sentence`,
`learn::learn_edit`), `flow-ffi` (`effective_category`, `learn_edit`),
`flow-router` (`learn::{Candidate, learn_edit}`), `flow-replay`
(`effective_category`), the probe runners under `tools/probes/prototypes/`
(`apply`). `Candidate` crosses the JNI boundary as JSON with the keys
`original`, `corrected`, `looks_like_name` (the Kotlin decoder treats a
missing `looks_like_name` as false).

`showed_prompt` stays in `PostOutput` for source compatibility and is
always false. Everything else in the old `postprocess` module tree
(`PostInput`, `process`, `process_with`, the `android`, `apps`, `context`,
`cursor_tag`, `press_enter`, `slack`, `stacked` modules and their
functions) had no caller outside the module and its ground-truth harness,
and is not part of the contract. New internal structure is free.

## 3. Pipeline (`apply_with`)

Inputs used: `cfg.platform`, `cfg.dictionary`, `cfg.user_names`,
`cfg.styles`, `cfg.all_lowercase`, `cfg.press_enter_enabled`,
`cfg.email_signature`; `ctx.app_id`, `ctx.url_host`, `ctx.app_category`,
`ctx.before`, `ctx.selected`, `ctx.after`, `ctx.screen_words`,
`ctx.context_known`. (`ctx.file_names` is no longer read, see §10.)

Steps, in order:

1. **Category and style.** `category = effective_category(platform, ctx)`;
   `style = cfg.styles.for_category(category)`.
2. **Whole-utterance snippet** (§4.3, Windows only): when
   `m.whole_snippet(raw_asr)` is `Some(body)`, return `body` exactly as
   stored (no other step, no trailing space, `press_enter` false).
3. **Press Enter** (§8, Windows only): remove a trailing spoken "press
   enter" and remember it.
4. **All-lowercase** (`cfg.all_lowercase`): the whole text is lowercased
   (including "I" → "i"). Later steps never capitalise except §5.6 over
   a capitalised selection, so the output stays lowercase except for
   text a dictionary replacement inserts.
5. **Dictionary replacements** (§4): `m.find_and_replace(text)`.
6. **Fit to the cursor** (§5), skipped for the apps in §5.7: leading
   space, case, final punctuation.
7. **Style punctuation** (§6).
8. **Trailing space** (§5.4), not when Press Enter fired.
9. **Email signature** (§7, Windows only).

The result text may be empty (a dictation that was only "press enter").

## 4. Dictionary (`DictMatcher`)

A `DictEntry` is `{word, replacement: Option<String>, is_snippet}`.

### 4.1 Which entries act

* An entry with no replacement (a vocabulary word) never rewrites text;
  it only biases the speech model and counts as a proper noun (§5.3).
  Observed: "deploy the kubernetes job." with vocabulary word
  "Kubernetes" stays as is.
* An entry with a replacement rewrites every whole-word occurrence of its
  `word` (the key). Snippets (`is_snippet`) also rewrite inline.
* The key is trimmed; an entry whose trimmed key is empty is ignored.
* `new(personal, team)`: both lists act; when a personal and a team entry
  have the same key (case-insensitive), the personal one wins. The engine
  always passes an empty team list.
* `is_empty()`: no entry rewrites anything.

### 4.2 Matching (`find_and_replace`)

* Case-insensitive (Unicode simple case folding). The replacement is
  inserted exactly as stored, whatever the case of the match:
  "cubernetes" / "Cubernetes" / "CUBERNETES" → "Kubernetes".
* Whole words: the character before the match and the character after it
  must not be a letter, digit or combining mark (any script), so
  "cubernetesx" and "écafé" do not match "cubernetes" / "café".
  A key may contain any characters ("c++", "node.js", "café"); they match
  literally. A key that starts or ends with a non-word character
  ("c++") still needs a non-word-character (or text edge) neighbour on the
  side where the key has a word character.
* Contraction guard: a match followed by an apostrophe (`'` or `’`) and
  the letter "t" is not replaced, so key "don" leaves "don't" alone but
  turns "ask don today" into "ask Don today". A possessive is replaced:
  "the cubernetes's job" → "the Kubernetes's job".
* Spaces inside a key match one or more whitespace characters in the text
  (**Change**: the old code needed exactly the key's spacing; "zen  trix"
  now matches "zen trix").
* Overlaps: at each position the longest key wins, whatever the entry
  order ("zen trix hub" beats "zen trix"). Replacements do not chain: text
  a replacement inserted is not matched again.
* The replacement is literal: "$1 fee" inserts "$1 fee".

Examples (observed, kept): "deploy the cubernetes job." → "deploy the
Kubernetes job."; "is it cubernetes?" → "is it Kubernetes?";
"cubernetes and cubernetes." → "Kubernetes and Kubernetes.";
"email me at c plus plus." → "email me at C++."; "we like c++ a lot." →
"we like C plus plus a lot."; "open node.js now." → "open Node.js now.";
"room 42 is free." with key "42" → "room forty-two is free.";
key "  hello  " → "Hi" turns "hello there." into "Hi there.".

### 4.3 Whole-utterance snippet (`whole_snippet`)

`whole_snippet(text)` returns the snippet body when the whole `text`,
normalised, equals a snippet key, normalised; else `None`. Normalising
= lowercase, drop every character that is not a letter, digit or space,
collapse runs of whitespace to one space, trim. Only `is_snippet`
entries count; when two snippet keys normalise the same, the personal
one, then the first one, wins.

Observed, kept: with snippet "my address" → "12 Elm Road", each of
"my address", "My address", "my address.", "  my address  ",
"my  address", "My Address!", "MY ADDRESS?" returns the body;
"say my address" and "my addresses" return `None`; a non-snippet
replacement key returns `None`.

In `apply_with` (§3 step 6) the snippet is looked up on the **raw
transcript**, on Windows only. The engine also uses `whole_snippet` to
skip spoken cleanup and the LLM for a snippet dictation.

Observed, kept: raw "my address" → output "12 Elm Road" (no trailing
space, whatever the cursor context); a body with a newline is inserted as
is. On Android the snippet is not taken whole; the text goes through the
normal steps (the inline replacement still turns "My address." into
"12 Elm Road" plus a trailing space).

## 5. Fit to the cursor

The text was dictated into a field that may already hold text around the
cursor: `before` (text before the cursor, up to 500 chars), `selected`
(the selection the text replaces) and `after`. This step makes the
dictation read as part of that text. Let `t` be the text with leading and
trailing whitespace trimmed (an all-whitespace text becomes empty and the
step returns it unchanged, empty). **Change**: the old code returned a
single space for an all-whitespace text after "x "; it now returns "".

Definitions:

* *sentence end*: `.`, `!`, `?`, `…`, and the CJK marks `。！？`.
* *CJK*: Han, Hiragana, Katakana, Hangul and the CJK punctuation block
  plus full-width forms.
* *opening mark*: `(`, `[`, `{`, `"`, `'`, `“`, `‘`, `«`, `¿`, `¡`.
* `b` = `before` with trailing spaces and tabs removed; `b_last` its last
  character. `a` = `after` with leading spaces and tabs removed;
  `a_first` its first character.

### 5.1 Leading space

Prepend one space when `before` is not empty and its last character is
not whitespace, not an opening mark, not `-`/`/`/`@`, and neither it nor
the first character of `t` is CJK.

Observed, kept: "I think" → " the parcel…"; "Well," → " the parcel…";
"Done." → " The parcel…"; "42" → " items."; "wait..." / "wow!" → a space
and the capital kept; "I think " → no space; "(" / "“" / "He said \"" →
no space; "well-" / "a/" / "mail@" → no space; "你好" + "Hello there." →
no space.

### 5.2 Lower-case start (mid-sentence)

Let `b` = `before` with trailing spaces (U+0020) and non-breaking spaces
removed (not tabs or newlines). Lower-case the first letter of `t` when
`b` is not empty and its last character is a letter or digit (any
script, CJK included), or one of `, ; : - – — /`. So "Note: " and
"so - " and "a " lower the text; a sentence end ("Done. ",
"Really? ", "Wait... "), a newline, a tab, an opening mark ("(", "\""),
`@` or an empty `before` leave it. The first word is left alone when:

* `starts_with_proper_noun` (§5.3) is true, or
* it is not a plain capitalised word: it has another upper-case letter
  after the first ("NASA", "IPhone", "McKay"), or starts with a
  non-letter ("42 people came." stays).

Only the first letter of the text changes; later sentences keep their
case ("Done. Next step." after "ok " → "done. Next step. ").

**Change**: the old code lowered the first letter of all-caps and
mixed-case words ("NASA called." after "and " became "nASA called.",
"IPhone" became "iPhone"); such words now stay as they are.

This step never upper-cases: "the parcel arrived early." after "Done. "
stays lower-case.

### 5.3 `starts_with_proper_noun(text, proper, dict)`

True when the first whitespace-separated word of `text`, with
surrounding punctuation removed, is:

* the pronoun "I" or starts with "I'" / "I’" ("I'm", "I’ll"), or
* equal (case-sensitive) to an entry of `proper` or to the first word of
  a multi-word entry of `proper`, or
* equal (case-sensitive) to the first word of a dictionary entry's `word`
  or of its `replacement`.

`apply_with` passes `proper = cfg.user_names ++ ctx.screen_words` and
`dict = cfg.dictionary.entries`. Observed, kept: "Mara called." after
"and " keeps "Mara" when "Mara" is a user name, else becomes "mara
called."; screen word "Quillon" keeps "Quillon is down."; vocabulary word
"Kubernetes" keeps "Kubernetes failed."; "I left early." and
"I'm on my way." keep "I".

### 5.4 Final punctuation and trailing space

Let `continues` = `a` is not empty and `a_first` is a lower-case letter
or one of `, ; : . ? !`. When `continues`, remove the final run of
sentence-end characters from `t` ("Is it ready?" before " now" →
"is it ready"; "The parcel arrived early." before ", then" → "…early").

Trailing space: append one space when the field is otherwise ready for
more typing:

* `after` is empty, or
* `after` starts directly (no whitespace) with a letter or digit,

and `t` does not end with a CJK character or CJK sentence end, and the
context is known or the platform is Windows (§5.5).

No space when `after` starts with whitespace (including a newline) or
with punctuation, a closing mark or a quote.

Observed, kept: empty field → "The parcel arrived early. "; after
"and then" → "The parcel arrived early " (period dropped, space added);
after " and then" → "The parcel arrived early"; after "Then we left." →
"The parcel arrived early. "; after "42" → "The parcel arrived early. ";
after "." / "," / ";" / "?" → "The parcel arrived early"; after ")" or
"\"" → "The parcel arrived early."; after " " → "The parcel arrived
early."; CJK "世界很好。" → no space.

**Change**: after "\nNext line" the old code added a space before the
newline; no space now (a newline is whitespace). After ") more" the old
code dropped the period; now a closing mark keeps it. The CJK marks
`。！？` are sentence ends here too: "好的。" before "x" → "好的" (the old
code kept "。").

The trailing space is intended: the next dictation into the same field
must not run into this one. `plain_text` keeps it; shells insert it.

### 5.5 Unknown context, Android

When `ctx.context_known` is false (the shell could not read the field)
and the platform is Android, no trailing space is added (the field may
already have one). On Windows an unknown context is treated as an empty
field. Observed, kept: Windows "The parcel arrived early. ", Android
"The parcel arrived early.".

### 5.6 Selection

When `selected` is not empty the text replaces it, and takes the
selection's shape instead of following `before`/`after` (§5.1–§5.5 do
not apply):

* Start: when the trimmed selection starts with a lower-case letter, the
  first letter of `t` is lowered (same exceptions as §5.2); when it
  starts with an upper-case letter, the first letter of `t` is
  upper-cased; otherwise unchanged.
* End: when the trimmed selection does not end in a sentence end, the
  final run of sentence-end characters of `t` is removed; when it does,
  `t` keeps its own ending.
* The selection's own leading and trailing whitespace is put around the
  text; no other space is added.
* §6 style punctuation is not applied (the selection's shape wins).

Observed, kept: selection "Bad words." → "New words."; "Bad words" →
"New words"; "bad words." → "new words."; "x" → "new words"; "BAD" →
"New words"; "Bad words?" → "New words."; "New words. More here." over
"bad" → "new words. More here"; "Is it new?" over "bad" → "is it new";
selection " bad words " → " new words ".
**Change**: over " Bad words. " the old code returned " New words "
(it looked at the untrimmed end); now " New words. ".

### 5.7 Apps that manage their own spacing

Cursor fitting (§3 step 6) and the trailing space (step 8) are skipped,
so the trimmed text is inserted without case, space or punctuation
changes from the context (style punctuation still applies), for:

* Notion: Windows app id `notion.exe`, Android package `notion.id`, or a
  browser on host `notion.so` / any `*.notion.so` / `notion.site`;
* this app's own windows: Windows app ids `flow-windows.exe`,
  `murmur.exe`, `murmur.exe`; Android package `dev.flowclone` and
  its flavour suffixes (`dev.flowclone.*`).

**Change**: the old skip fired only on the exact host `notion.so`; it now
covers the Notion desktop app, `www.notion.so` and this app's own fields.

## 6. Style punctuation

`style` is the per-category style (§3 step 1):

| Style | Effect |
|---|---|
| `formal` | none |
| `casual`, `very_casual` | drop the final `.` of the text, any category (not `?`, `!`, `…` or `...`) |
| `default`, `excited` | in a chat category (`AppCategory::is_chat`), drop the final `.` when the text has at most 2 sentences |

A sentence, for the count, ends at `.`, `!`, `?` or a newline followed by
more text; the count is of non-empty sentences ("Are you there? Call
me." = 2; "See you soon.\nBring the map." = 2).

Applied to the text after cursor fitting, before the trailing space: the
space stays ("See you soon "). Text ending in "..." keeps it; text ending
in a URL loses only the sentence period ("Visit example.com." → "Visit
example.com"). Not applied over a selection (§5.6).

Observed, kept: casual "See you soon. Bring the map. Call Mara first."
→ "…Call Mara first " in every category; default in WhatsApp "See you
soon. Bring the map." → "See you soon. Bring the map ", 3 sentences keep
the period, "Are you there? Call me." → "Are you there? Call me ";
formal keeps every period; "Are you coming?" and "That is great!" keep
their mark in every style.

**Change**: the old code decided "chat" for `default`/`excited` by its
own app-id lists and ignored the category the shell sent, so a
personal-chat bucket with an unlisted app id kept the period while the
same app id with another bucket lost it. The category now decides, so
the user's per-app buckets apply.

## 7. Email signature (Windows)

When `cfg.email_signature` is a non-empty string `sig`, the category is
`Email`, the context is known, and the dictation reads like an email
body, append `"\n\n" + sig` to the text, after removing the trailing
space.

* Reads like an email body: it starts with a greeting ("hi", "hello",
  "hey", "dear", case-insensitive, followed by a space, comma or "!"), or
  it has at least 10 words that contain a letter or digit.
* Not when the signature is already there: `sig` with leading dashes,
  em dashes and spaces removed, lowercased, occurs in `before`,
  `selected`, `after` or the text (lowercased, with non-breaking spaces
  and `&nbsp;` read as spaces).

Observed, kept (with trailing-space change): "Hi Tom, thanks." in Outlook
with signature "— Ann" → "Hi Tom, thanks.\n\n— Ann"; "Thanks for that."
gets no signature; 10 words get it, 9 do not.

**Change**: (1) the category, not an app-id list, decides "email" (the
old code missed the new Outlook `olk.exe` and ignored a shell-set Email
category); (2) the old output kept the trailing space before the
newlines ("thanks. \n\n— Ann").

## 8. Press Enter (Windows)

When `cfg.press_enter_enabled` (default true) and the text ends with the
spoken command "press enter" (case-insensitive, as its own words,
optionally followed by `.`, `!` or `?`), the command is removed and
`press_enter` is true. The punctuation that followed the command moves to
the text before it when that text ends in a letter or digit:
"Send it press enter." → "Send it."; "Send it press enter!" → "Send it!";
"Send it. Press enter." → "Send it."; "Send it, press enter" → "Send
it,"; "press enter" → "" (Enter only). Also when the raw transcript ends
with the command but the cleaned text dropped it, `press_enter` is true
and the text is kept.

With `press_enter`, no trailing space is added (the message is sent).
**Change**: the old code added one in the raw-only case.

Not a command: "Press enter now.", "Send it press the enter.", "hit
enter", "press return". Disabled (`press_enter_enabled = false`) or on
Android: the words stay ("Send it press enter. ") and `press_enter` is
false, also in the raw-only case.

Order: the command is removed before the cursor and style steps see the
text, so "Send it press enter." after " more" → "Send it", and casual
"Send it press enter." → "Send it" (both with `press_enter`);
"Send it.\nPress enter." → "Send it.".

## 9. App category (`effective_category`)

`ctx.app_category` wins when it is not `Other`. Otherwise the app id
(Windows process name, case-insensitive, with or without `.exe`; Android
package name, exact) and the URL host (for browsers; a host matches
itself and its subdomains) map to a category from this module's own
lists, written for this project:

| Category | Windows app ids | Android packages | Hosts |
|---|---|---|---|
| Email | outlook, olk, thunderbird, betterbird, mailbird, mailspring, hxoutlook | com.google.android.gm, com.microsoft.office.outlook, ch.protonmail.android, com.samsung.android.email.provider, com.fsck.k9, net.thunderbird.android, com.yahoo.mobile.client.android.mail | mail.google.com, outlook.live.com, outlook.office.com, outlook.office365.com, mail.proton.me, mail.yahoo.com |
| WorkChat | slack, ms-teams, teams, zoom, mattermost | com.slack, com.microsoft.teams, us.zoom.videomeetings, com.mattermost.rn | app.slack.com, teams.microsoft.com, teams.live.com |
| PersonalChat | whatsapp, whatsapp.root, telegram, discord, signal, messenger, viber, line, skype, element | com.whatsapp, com.whatsapp.w4b, org.telegram.messenger, org.thoughtcrime.securesms, com.discord, com.facebook.orca, com.instagram.android, com.viber.voip, jp.naver.line.android, com.google.android.apps.messaging, com.samsung.android.messaging, im.vector.app | web.whatsapp.com, web.telegram.org, discord.com, messenger.com |
| AiPrompt | chatgpt, claude, perplexity, cursor, windsurf, code | com.openai.chatgpt, com.anthropic.claude, ai.perplexity.app.android, com.google.android.apps.bard | chatgpt.com, chat.openai.com, claude.ai, gemini.google.com, perplexity.ai |
| Terminal | windowsterminal, wt, cmd, powershell, pwsh, conhost, alacritty, wezterm-gui, mintty, putty | com.termux, org.connectbot, com.server.auditor.ssh.client | — |
| Document | winword, notion, obsidian, onenote, soffice, swriter | com.google.android.apps.docs.editors.docs, com.microsoft.office.word, notion.id, md.obsidian | docs.google.com, notion.so, notion.site |

A Windows app id is looked up only on Windows, an Android package only on
Android; hosts on both. Unknown → `Other`.

Observed, kept: Slack/Outlook/Thunderbird/WhatsApp/Telegram/Discord/
Signal/terminals/ChatGPT/Claude/Cursor/VS Code and the Android packages
probed map as before. **Change**: `olk.exe`, `ms-teams.exe`/`Teams.exe`
(new Outlook, Teams), `com.slack` (Android Slack) were `Other` and now
have their category; Word, Notion and Google Docs are `Document` (were
`Other`); LinkedIn and Instagram: Instagram stays personal chat,
LinkedIn stays `Other`.

## 10. Removed behaviour

* **IDE `@file` tagging** (Cursor/Windsurf: a spoken file name became
  "@file" with paste markers). It needs `ctx.file_names`, which no shell
  fills (Windows sends an empty list, Android has none), so it never ran
  in the product. `paste.rs` keeps its `{{BREAK}}`/`{{TAB}}` marker
  support.
* **Slack "at Name" → "@Name"** tagging: gated on macOS app ids that no
  supported platform reports, and off on the engine path, so it never ran.
* **Stacked-message splitting** for personal chats: never changed any
  probed output.
* **Android shell helpers** (`postprocess::android`: compose decision,
  placeholder detection, bubble policy, excluded fields): no caller; the
  Android app implements its own in Kotlin.
* The JSON app lists in `crates/flow-core/data/` are replaced by the
  tables in §9.

## 11. Edit learning (`learn_edit`)

`learn_edit(pasted, edited, dict)`: the user edited the text the app
inserted (`pasted`) into `edited`; return up to `MAX_CANDIDATES`
dictionary candidates, in text order.

1. Align the two word sequences with `align::align_words` (word-level
   edit distance; words compare exactly, after trimming surrounding
   punctuation, so a case change is a substitution).
2. Group consecutive substitutions into runs (a run ends at a kept word,
   an insertion or a deletion). A run whose original side has more than
   `MAX_RUN_WORDS` words is a rewrite, not vocabulary: dropped.
   Insertions and deletions alone produce nothing.
3. A run becomes `Candidate{original, corrected}` = the run's original
   and edited words joined by one space, each word with surrounding
   punctuation trimmed. It is dropped when:
   * original and corrected are equal ignoring case and the run is one
     word at the start of a sentence (sentence case, not a correction:
     "zentrix is down" → "Zentrix is down" gives nothing; mid-sentence
     "the zentrix" → "the Zentrix" gives a candidate);
   * every corrected word is a stopword (articles, pronouns, common
     prepositions, conjunctions, auxiliaries: "the" → "a");
   * corrected contains an apostrophe contraction ("do not" → "don't") or
     original/corrected differ only in punctuation;
   * corrected (case-insensitive) is already a dictionary word or
     replacement target, or original is already a replacement key;
   * **Change**: corrected has no letter ("42" → "43" was a candidate).
4. `looks_like_name` = `looks_like_name(corrected, sentence_initial)`,
   where `sentence_initial` = the run starts the text or follows a word
   ending in a sentence end.
5. **Change**: duplicates (same original and corrected) are merged into
   the first, with `looks_like_name` true if any occurrence had it.
6. At most `MAX_CANDIDATES`, the first ones in text order.

`looks_like_name(corrected, sentence_initial)`: every word starts with an
upper-case letter and contains a lower-case letter (so not "I", not
"NASA", not "42"), and not (`sentence_initial` and a single word).

Examples (this repo's tests, kept; brand names replaced with invented
ones): "I use zentrix hub daily" → "I use Zentrix Hub daily" gives
("zentrix hub", "Zentrix Hub", name); "I met gina at the station" → "I
met Jenna …" gives ("gina", "Jenna", name); "deploy the cubernetes job" →
"deploy the kubernetes job" gives ("cubernetes", "kubernetes", not a
name); "call doc marlo today" → "call Dr marlowe today" gives ("doc
marlo", "Dr marlowe", not a name); 5 corrected names → 4 candidates;
"please call me back tomorrow morning at nine" → an unrelated sentence
gives nothing; "hello world" → "Hello, world." gives nothing.

Observed, kept: "ask gina about lumo" → "ask Jenna about Lumio" gives
two name candidates; "go to the lumo" → "go to a Lumio" gives ("the
lumo", "a Lumio", not a name); "see you at lumo" → "see you at Lumio
tomorrow then bye" gives ("lumo", "Lumio tomorrow then bye") — the
Windows shell bounds the edit first (`learn::bound_edit` in the app), so
the core does not guess where the dictation ended.

## 12. Tests

* Unit tests next to the new code cover every "Observed, kept" and
  "Change" example in this spec, with made-up text only.
* `crates/flow-core/tests/vectors.rs` and `tests/vectors/*.json` are
  removed: their expectations were produced by another product's code,
  not by this project's decisions.
* `learn.rs`'s own tests stay (they are this project's), with the
  example brand name replaced by an invented one.
* The engine-level tests (`tests/session.rs`, `tests/d*_*.rs`,
  `tests/spoken_bench.rs`, `flow-router` and `flow-windows` tests) must
  pass unchanged, except where a test asserts a behaviour this spec
  marks **Change**; such an edit is listed in the commit. Edited:
  `tests/api_additions.rs` (IDE tagging test → "file names are not
  tagged", §10) and the `flow-protocol` fixtures `server_result*.json`
  (Android Slack, work-chat category: the period now drops, §6).
* Parity check: `tools/probes/prototypes/cleanroom_2026_11_09/` runs the
  probe cases and the deterministic formatting eval on the old and the
  new code; every difference must be a **Change** row.
