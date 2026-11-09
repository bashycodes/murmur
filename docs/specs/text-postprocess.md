# Spec: text post-processing and edit learning

Status: the behaviour contract for `crates/murmur-core/src/postprocess/`
(minus `spoken*.rs` and `terminal.rs`, which are specified by their own
module docs and tests) and `crates/murmur-core/src/learn.rs`. Written
2026-11-09 for a from-scratch rewrite of those modules.

Sources: the public API and its callers (`engine.rs`, `murmur-ffi`,
`murmur-router`, the Windows and Android shells), `docs/DECISIONS.md`,
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
// crates/murmur-core/src/postprocess/mod.rs
pub mod spoken;      // unchanged
pub mod terminal;    // unchanged
pub use DictMatcher; // see §4

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostOutput { pub text: String, pub press_enter: bool }

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

// crates/murmur-core/src/learn.rs
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
`learn::learn_edit`), `murmur-ffi` (`effective_category`, `learn_edit`),
`murmur-router` (`learn::{Candidate, learn_edit}`), `murmur-replay`
(`effective_category`), the probe runners under `tools/probes/prototypes/`
(`apply`). `Candidate` crosses the JNI boundary as JSON with the keys
`original`, `corrected`, `looks_like_name` (the Kotlin decoder treats a
missing `looks_like_name` as false).

`PostOutput.showed_prompt` (always false, never read) is removed.
Everything else in the old `postprocess` module tree
(`PostInput`, `process`, `process_with`, the `android`, `context`,
`cursor_tag`, `press_enter`, `slack`, `stacked` modules and their
functions) had no caller outside the module and its ground-truth harness,
and is not part of the contract. `pub mod apps` (`classify`, `AppInfo`,
over the shared app groups) stays: §9 uses it and
`tests/app_groups_vectors.rs` reads `classify(..).bucket`. New internal
structure is free.

## 3. Pipeline (`apply_with`)

Inputs used: `cfg.platform`, `cfg.dictionary`, `cfg.user_names`,
`cfg.styles`, `cfg.all_lowercase`, `cfg.press_enter_enabled`,
`cfg.email_signature`; `ctx.app_id`, `ctx.url_host`, `ctx.app_category`,
`ctx.before`, `ctx.selected`, `ctx.after`, `ctx.screen_words`,
`ctx.context_known`. (`ctx.file_names` is not read, see §10.)

Steps, in order:

0. **Empty text.** When `text` is empty, return `""` (`press_enter`
   false), even when `raw_asr` is a snippet key. Observed, kept.
1. **Category and style.** `category = effective_category(platform, ctx)`;
   `style = cfg.styles.for_category(category)`. Separately,
   `listed = apps::classify(platform, ctx.app_id, ctx.url_host)` (§9),
   which ignores `ctx.app_category`: it decides "chat" for §6, "email"
   for §7 and the skips of §5.7.
2. **Whole-utterance snippet** (§4.3, Windows only): when
   `m.whole_snippet(raw_asr)` is `Some(body)`, the text becomes `body`
   exactly as stored and steps 3–8 are skipped (no case, space or
   punctuation change, no trailing space, `press_enter` false). Step 9
   (signature) still runs on it.
3. **Press Enter** (§8, Windows only): remove a trailing spoken "press
   enter" and remember it.
4. **All-lowercase** (`cfg.all_lowercase`): the whole text is lowercased
   (including "I" → "i"). Later steps never capitalise except §5.6 over
   a capitalised selection, so the output stays lowercase except for
   text a dictionary replacement inserts.
5. **Dictionary replacements** (§4): `m.find_and_replace(text)`.
6. **Fit to the cursor** (§5): leading space, case, final punctuation;
   or a skip of §5.7.
7. **Style punctuation** (§6).
8. **Trailing space** (§5.4), not when Press Enter fired, not after a
   §5.7 skip.
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

In `apply_with` (§3 step 2) the snippet is looked up on the **raw
transcript**, on Windows only. The engine also uses `whole_snippet` to
skip spoken cleanup and the LLM for a snippet dictation.

Observed, kept: raw "my address" → output "12 Elm Road" (no trailing
space, whatever the cursor context; §7 may append the signature); a body with a newline is inserted as
is. On Android the snippet is not taken whole; the text goes through the
normal steps (the inline replacement still turns "My address." into
"12 Elm Road" plus a trailing space).

## 5. Fit to the cursor

The text was dictated into a field that may already hold text around the
cursor: `before` (text before the cursor, up to 500 chars), `selected`
(the selection the text replaces) and `after`. This step makes the
dictation read as part of that text. Let `t` be the text with leading and
trailing whitespace trimmed. An all-whitespace text becomes `""` and the
whole of §5–§7 is skipped. **Change**: the old code returned a single
space for an all-whitespace text; it now returns `""` (no stray space).

The context is used as the shell sent it, also when `ctx.context_known`
is false (§5.5 only changes the trailing space).

Definitions:

* *space*: U+0020 only. *nbsp*: U+00A0. Tabs and newlines are neither.
* *CJK*: Han, Hiragana, Katakana, Hangul and the CJK punctuation block
  plus full-width forms.
* *emoji*: a character with the Unicode `Extended_Pictographic` property
  (this includes ©, ™, ★, ✅, ❤ but not °), a variation selector U+FE0F,
  a skin-tone modifier U+1F3FB–U+1F3FF, or a regional indicator
  U+1F1E6–U+1F1FF (flags).
* *closing quote*: `"`, `'`, `’`, `”`.

### 5.1 Leading space

Only when `selected` is empty, `before` is not empty and `t` is not
empty. Let `c` be the last character of `before` (not trimmed). Prepend
one space when the first character of `t` is not CJK and `c` is one of:

* a letter or digit that is not CJK;
* `,` `.` `;` `:` `!` `?` `)` `]` `}` `%` `&` `+` `>` `|` `*` `=` `~`;
* a closing quote, when the character before it exists and is not
  whitespace ("x\"" yes; "\"", "a \"", "He said \"" no);
* an emoji;
* `-`, when it is the whole of `before` or the character before it is
  whitespace ("-", "x -" yes; "a-", "well-" no).

Anything else gets no space: whitespace (space, nbsp, tab, newline),
`(` `[` `{` `<` `«` `»` `¿` `¡` `‘` `“`, `—` and `–` (anywhere),
`^` `•` `…` (the single ellipsis character; "..." ends in `.` and gets
the space), `$` `#` `_` `@` `/`, an attached `-`, and CJK.

Observed, kept: "I think" → " the parcel…"; "Well," → " the parcel…";
"Done." → " The parcel…"; "42" → " items."; "wait..." / "wow!" → a space
and the capital kept; "x)" / "x]" / "x\"" / "x😀" / "50%" → a space;
"I think " → none; "(" / "“" / "He said \"" → none; "well-" / "a/" /
"mail@" / "x—" / "x…" / "$" → none; "你好" + "Hello there." → none.

### 5.2 Lower-case start (mid-sentence)

Only when `selected` is empty. Decide from `before`:

1. Remove trailing spaces and nbsp from `before`.
2. Then repeatedly remove from the end any run of these *transparent*
   characters: space (not nbsp, tab or newline), `,` `;` `:` `-` `—`
   `/` `(` `)` `[` `]` `{` `}`, emoji, and a closing quote that is
   immediately preceded by a non-whitespace character (`x”` → `x`,
   `a ”` stays).
3. Lower-case when what remains is not empty and its last character is a
   letter or digit (any script, CJK included).

So the text is lowered after "Note: ", "Well, ", "so - ", "x - ",
"a- ", "a/ ", "wait — ", "a—", "see (", "x(", "x ( ", "done) ",
"x;) ", "x)]", "x [", "x {", "great 😀 ", "x©", "x\" ", "x’",
"“x” ", "x "; and left after a sentence end ("Done. ", "Done. (",
"Done.) ", "Done. - ", "Done. — ", "! ("), after only transparent
characters ("(", "( (", "- ", ", ", ": ", "— ", "😀 ", "\" "), after a
newline or tab ("Done.\n(", "x\n) ", "Line one\n", "a\t"), after a quote
that follows whitespace or opens ("a ’", "a \"", "x‘ ", "x“ ", "He said
\"", "“"), after `«` `»`, the en dash `–` ("a –", "a–"), `$ # _ * = ~ %
& + < > | ^ • … @ ¿ ¡ °`, and after "x (" (nbsp is only stripped
in step 1).

Lowering changes only the first character of `t`, and only when it is an
upper-case letter. The first word is left alone when:

* `starts_with_proper_noun` (§5.3) is true, or
* it is an all-caps word: ignoring a trailing possessive `'s` / `’s`, it
  has at least two letters and every letter in it is upper-case ("NASA",
  "OK", "AI", "CEO", "TV", "U.S.", "I.B.M.", "NASA's", "USA's").

Every other capitalised first word gets only its first letter lowered,
also when later letters are upper-case: "IPhone" → "iPhone", "EBay" →
"eBay", "McKay" → "mcKay", "JavaScript" → "javaScript", "PhD" → "phD";
single letters too: "A plan" → "a plan", "X is" → "x is".

**Change**: the old code also lowered all-caps words ("NASA called."
after "and " became "nASA called."); they now keep their case.

Only the first letter of the text changes; later sentences keep their
case ("Done. Next step." after "ok " → "done. Next step. "). This step
never upper-cases: "the parcel arrived early." after "Done. " stays
lower-case.

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

Only when `selected` is empty.

*Continues*: remove leading spaces and nbsp from `after` (not tabs), then
skip a run of spaces, `-`, `—`, `(` `)` `[` `]` `{` `}` and `/`. The
dictation *continues* into `after` when the next character is a
lower-case letter or one of `, ; : . ? !`. When it continues, remove the
final run of `.`, `!`, `?` and `…` from `t` (not the CJK `。！？`).

Continues: "and then", " and then", " later", "— and", " — and",
"—and", "-and", "- and", "-- and", "—  and", "— (and)", "— ,", "(and",
"( and", ") and", ") more", "{x", "]x", "[x", "/x", ": x", "; more",
"... and", ".", ",", "?", " and". Does not: "– and" (en dash),
"— And", "- And", "Then", "— 5", "-5", "-", "—", "— ", "…and",
"\" and", "’ and", "😀 and", "$5", "_x", "@x", "\tand", "—\tand",
"-\nand", "\nNext line", " Next".

*Trailing space*: append one space when

* `after` is empty, or
* the first character of `after` is a letter or digit, or
* `after` starts with `-` followed directly by a space ("- and",
  "- And", "-  and"),

and `t` does not end with a CJK character, Press Enter did not fire
(§8), and the context is known or the platform is Windows (§5.5).

Observed, kept: empty field → "The parcel arrived early. "; after
"and then" → "The parcel arrived early " (period dropped, space added);
after " and then" → "The parcel arrived early"; after "Then we left." →
"The parcel arrived early. "; after "42" → "…early. "; after "." / "," /
";" / "?" / "— and" → "…early"; after ")" / "\"" / "—" / "😀" / "…" /
"$5" / "\tand" → "…early."; after " " → "…early."; after "- And" →
"…early. "; after "- and" → "…early "; CJK "世界很好。" before "再见"
→ "世界很好。"; "好的。" before "x" → "好的。".

**Change**: after "\nNext line" the old code added a space before the
newline ("…early. \nNext line"); now no space is added before a newline
("…early."), since the space would end a line.

The trailing space is intended: the next dictation into the same field
must not run into this one. `plain_text` keeps it; shells insert it.

### 5.5 Unknown context

When `ctx.context_known` is false (the shell could not read the field),
`before`, `selected` and `after` are still used as sent (§5.1–§5.4, §6).
On Android no trailing space is added (the field may already have one);
on Windows the trailing space rule is unchanged. Observed, kept: empty
context, Windows "The parcel arrived early. ", Android "The parcel
arrived early."; Android with `before` "ok " lowers the first letter; Android chat with `before` "Ok. " keeps the period.

### 5.6 Selection

When `selected` is not empty the text replaces it, and takes the
selection's shape instead of following `before`/`after` (§5.1–§5.4 do
not apply):

* Start: when the trimmed selection starts with a lower-case letter, the
  first letter of `t` is lowered (same exceptions as §5.2); when it
  starts with an upper-case letter, the first letter of `t` is
  upper-cased; otherwise unchanged.
* End: when the trimmed selection does not end in `.`, `!`, `?`, `…` or
  one of `。！？`, the final run of those characters is removed from `t`;
  when it does, `t` keeps its own ending.
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

### 5.7 Apps that manage their own text

`app = apps::classify(platform, ctx.app_id, ctx.url_host)` (§9).

*Notion*: when `app.is_notion`, `selected` is empty, `before` is not
empty, `after` is empty, and either `before` ends with "..." or
`before.split(' ')` has at most 2 pieces (empty pieces count: "so " is
2 pieces, "so then " is 3, "so then" is 2), §5.1–§5.4 are skipped: `t`
is used as is and no trailing space is added. Style punctuation (§6)
still applies.

Observed, kept: on host `notion.so` (also `notion.so/page`) or Android
package `notion.id`: "so " → "The parcel arrived early."; "so then" and
"Done." → same; "we went to the shop..." → same; "so then " → "the
parcel arrived early. "; "we went to the shop... " → "The parcel arrived
early. "; "so " with `after` "x" → "the parcel arrived early "; empty
`before` → "The parcel arrived early. ". Not Notion: `www.notion.so`,
`Notion.so`, `*.notion.site`, the Windows app `Notion.exe`.

*This app's own windows* (Windows only): when `app.is_murmur_app`, §5.1–§5.6
are skipped whatever the context: `t` is inserted as is, with no
trailing space; Press Enter (§8) still applies, and style punctuation
(§6) applies only when `selected` is empty (over a selection the text
keeps its period, as in §5.6).
Observed, kept: `murmur.exe` / `murmur-windows.exe` with any `before`,
`after` or selection → "The parcel arrived early."; casual over the
selection "abc" → "The parcel arrived early."; casual → "The parcel
arrived early"; "Send it press enter." → "Send it." with `press_enter`.
Android `dev.murmur` is not special.

## 6. Style punctuation

`style` is the per-category style (§3 step 1). Only the final `.` of the
text can be dropped (never `?`, `!`, `…` or `...`).

| Style | Drops the final `.` when |
|---|---|
| `formal` | never |
| `casual` | the text has at most 10 sentences |
| `very_casual` | always |
| `default`, `excited` | the app is a chat app and the text has at most 2 sentences |

*Chat app*: `listed.is_messaging` (§9). The
category the shell sent (`ctx.app_category`) does not make an app a chat
app and does not stop one being one: Notepad with a personal-chat bucket
keeps the period; WhatsApp with an email bucket drops it.

*Sentence count*: split the text at every `.`, `!` and `?` and count
the pieces that are not empty after trimming. A newline is not a break:
"See you soon.\nBring the map." = 2, ten lines without a mark = 1;
"Are you there? Call me." = 2; "Wait... then go." = 2.

*Suppressed*: no rule drops the period when the current line of
`before` (the text after its last `\n`), the current line of `after`
(the text up to its first `\n`) or `selected` contains `.`, `!` or `?`.
"Ok. " / "Ok. " / "Ok.\t" before, or " Ok." after, keep the period;
"wait… " (the ellipsis character), "好。", "Note: " and "Ok.\r\n" (the
line after the newline is empty) do not.

Applied to the text after cursor fitting, before the trailing space: the
space stays ("See you soon "). Text ending in "..." keeps it; text ending
in a URL loses only the sentence period ("Visit example.com." → "Visit
example.com"). Not applied over a selection (§5.6).

Observed, kept: casual "See you soon. Bring the map. Call Mara first."
→ "…Call Mara first " in every app; casual with 11 sentences keeps the
period; default in WhatsApp "See you soon. Bring the map." → "See you
soon. Bring the map ", 3 sentences keep the period, "Are you there? Call
me." → "Are you there? Call me "; default in Notepad with a
personal-chat bucket keeps the period;
formal keeps every period; "Are you coming?" and "That is great!" keep
their mark in every style.

## 7. Email signature (Windows)

When `cfg.email_signature` is a non-empty string `sig`, the app is an
email app (`listed.app_type == Email`, §9), the context is known, and the text reads like an
email body, remove one trailing space from the text and append
`"\n\n" + sig`. The text here is the output of step 8, or the snippet
body from step 2.

* Reads like an email body: it starts with a greeting ("hi", "hello",
  "hey", "dear", case-insensitive, followed by a space, comma or "!"), or
  it has at least 10 words that contain a letter or digit.
* Not when the signature is already there: `sig` with leading dashes,
  em dashes and spaces removed, lowercased, occurs in `before`,
  `selected`, `after` or the text (lowercased, with non-breaking spaces
  and `&nbsp;` read as spaces).
* The shell's `ctx.app_category` does not count: Notepad with an email
  bucket gets no signature, Outlook with a work-chat bucket gets it.

Observed, kept: "Hi Tom, thanks." in Outlook with signature "— Ann" →
"Hi Tom, thanks.\n\n— Ann"; "Thanks for the update." gets no signature;
10 words get it, 9 do not; a whole-utterance snippet whose body starts
"Hi Tom, …" gets it, a short non-greeting snippet body does not; empty
text → "" (no signature).

**Change**: the old output kept the trailing space before the newlines
("thanks. \n\n— Ann"); it is removed now, since it would end a line.

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

## 9. App category

Classification comes from the shared app groups (`crate::app_groups`,
`crates/murmur-core/data/app-groups.json`, D81) through
`postprocess::apps::classify(platform, app_id, url_host) -> AppInfo`
(that module and `AppInfo`/`AppType`/`Bucket` are kept as they are; the
`app_groups_vectors` test reads `classify(..).bucket`). Post-processing
has no app lists of its own.

* `effective_category(platform, ctx)`: `ctx.app_category` when it is not
  `Other`, else `classify(..).category()`.
* `listed` (§3 step 1) is `classify(..)`, which ignores
  `ctx.app_category`: chat (§6) = `listed.is_messaging`; email (§7) =
  `listed.app_type == AppType::Email`; Notion and own-app (§5.7) =
  `listed.is_notion` / `listed.is_murmur_app`.
* Windows app ids are compared case-insensitively and without a trailing
  `.exe` ("Outlook.EXE" = "outlook"), as the Windows shell does. **Fix**:
  `classify` passed the id with `.exe` to the groups, so on Windows no
  app id matched (no signature in Outlook, no chat period drop in Slack,
  no own-app skip); it now strips `.exe` first.

Membership is whatever the app groups say (D81 widened it: e.g. new
Outlook `olk`, Teams, Android Slack and WeChat are now email / chat).

## 10. Removed behaviour

* **IDE `@file` tagging** (Cursor/Windsurf: a spoken file name became
  "@file" with paste markers). It needs `ctx.file_names`, which no shell
  fills (Windows sends an empty list, Android has none), so it never ran
  in the product. `ctx.file_names` stays in `FieldContext` because it is
  part of the client/server wire format. `paste.rs` keeps its
  `{{BREAK}}`/`{{TAB}}` marker splitting, which now has no producer in
  murmur-core (the markers only pass through when they are in the text,
  e.g. a snippet body).
* **Slack "at Name" → "@Name"** tagging: gated on macOS app ids that no
  supported platform reports, and off on the engine path, so it never ran.
* **Stacked-message splitting** for personal chats: never changed any
  probed output.
* **Android shell helpers** (`postprocess::android`: compose decision,
  placeholder detection, bubble policy, excluded fields): no caller; the
  Android app implements its own in Kotlin.
* `PostOutput.showed_prompt`: always false, no reader; removed.

## 11. Edit learning (`learn_edit`)

`learn_edit(pasted, edited, dict)`: the user edited the text the app
inserted (`pasted`) into `edited`; return up to `MAX_CANDIDATES`
dictionary candidates, in text order.

1. Align the two word sequences with `align::align_words` (word-level
   edit distance; words compare exactly, after trimming surrounding
   punctuation, so a case change is a substitution).
2. Group consecutive substitutions into runs (a run ends at a kept word,
   an insertion or a deletion). Insertions and deletions alone produce
   nothing.
3. A run becomes `Candidate{original, corrected}` = the run's original
   and edited words joined by one space, each word with surrounding
   punctuation trimmed. It is dropped when:
   * either side has more than `MAX_RUN_WORDS` words (a rewrite, not
     vocabulary): "lumo" → "one two three four five" and "a b c d e" →
     "Lumio" give nothing, 4 words give a candidate;
   * original and corrected are equal ignoring case and the run starts
     a sentence: the run is the first word of the text, or the word
     before it ends in `.`, `!` or `?` (a newline or colon does not
     count). This holds for any run length and either direction
     ("lumo hub one is here" → "Lumo Hub One is here" and "Lumo is
     here" → "lumo is here" give nothing; "ok lumo" → "ok Lumo",
     "ok\nlumo" → "ok\nLumo" and "ok: lumo" → "ok: Lumo" give one);
   * every corrected word is a stopword, compared case-insensitively.
     The stopwords are exactly: um uh like okay ok yes no just very
     there here so then not the a an and or but to of in on at for with
     is it this that i you we they he she my your be have do was are
     will can would should if what who me us him her them its our their
     these those which as by from were been has had does did could.
     ("his", "yeah", "well", "oh", "now", "really", "where", "am",
     "may", "er" are not stopwords.) Only the corrected side is checked:
     "we um fine" → "we Zentra fine" gives ("um", "Zentra");
   * either side contains an apostrophe (`'` or `’`): "dont" → "don’t",
     "lumo’s" → "Lumio" give nothing;
   * corrected (case-insensitive) is already a dictionary word or a
     replacement target. (An original that is already a replacement key
     is kept: with "lumo" → "Lumio" in the dictionary, "lumo" → "Lumia"
     gives a candidate.)
   * **Change**: corrected has no letter ("42" → "43" was a candidate;
     a number edit is not vocabulary).
4. `looks_like_name` = `looks_like_name(corrected, sentence_initial)`,
   where `sentence_initial` = the run starts a sentence as defined in
   step 3.
5. **Change**: duplicates (same original and corrected) are merged into
   the first, with `looks_like_name` true if any occurrence had it (the
   old code returned each occurrence, filling the four slots with
   repeats).
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
lumo", "a Lumio", not a name); "we zentra fine" → "we the Lumio fine"
gives ("zentra", "the Lumio"); "lumo is here" → "Lumio is here" gives
("lumo", "Lumio", not a name: sentence-initial single word); "see you
at lumo" → "see you at Lumio tomorrow then bye" gives ("lumo", "Lumio
tomorrow then bye")
— the Windows shell bounds the edit first (`learn::bound_edit` in the
app), so the core does not guess where the dictation ended.

## 12. Tests

* Unit tests next to the new code cover every "Observed, kept" and
  "Change" example in this spec, with made-up text only. They include
  "IPhone" → "iPhone" and "NASA" kept mid-sentence.
* `crates/murmur-core/tests/vectors.rs` and `tests/vectors/*.json` are
  removed: their expectations were produced by another product's code,
  not by this project's decisions.
* `learn.rs`'s own tests stay (they are this project's), with the
  example brand name replaced by an invented one.
* The engine-level tests (`tests/session.rs`, `tests/d*_*.rs`,
  `tests/spoken_bench.rs`, `murmur-router`, `murmur-protocol` and
  `murmur-windows` tests and fixtures) must pass unchanged, except where a
  test asserts a behaviour this spec marks **Change**; such an edit is
  listed in the commit. Edited: `tests/api_additions.rs` (IDE tagging
  test → "file names are not tagged", §10).
* Parity check: `tools/probes/prototypes/cleanroom_2026_11_09/` runs the
  probe cases and the deterministic formatting eval on the old and the
  new code; every difference must be a **Change** row.

## Change rows (summary)

1. All-whitespace text → `""` (§5).
2. All-caps first words keep their case mid-sentence (§5.2).
3. No trailing space before a newline in `after` (§5.4).
4. Selection end read from the trimmed selection (§5.6).
5. No space between the text and the signature's newlines (§7).
6. No trailing space when Press Enter fired from the raw transcript (§8).
7. Dictionary keys with spaces match any whitespace run (§4.2).
8. Learning: a letterless correction is no candidate; duplicates merge
   (§11).
9. Fix: Windows app ids match the app groups without `.exe` (§9).
