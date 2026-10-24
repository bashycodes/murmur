# Formatting gap vs Wispr: what we handle, what Wispr does, what to fix (2026-10-24)

**Question.** The user, after running Wispr side by side today: Wispr "gets it exactly to how
I was planning everything written". It turns "quote unquote" into quotation marks, turns "in
parentheses X" into "(X)", is more casual overall, and places periods and commas correctly. Ours
doesn't. Where is the gap, and which fix buys the most per millisecond?

**Answer (short).**
1. **Two of the biggest gaps cost 0 ms and are configuration or rules, not models.**
   * *Style.* Both shells send `formal` for every style bucket. With `formal`, the cursor-context step never
     drops the final period, so chat apps look stiff. Wispr Android defaults personal chat to
     **Casual**. Sending casual for chat buckets moves the casual category from 1/7 to 7/7, and
     the deterministic pipeline overall from 44 % to 59 %.
   * *Spoken commands.* Nothing in the pipeline handles quote / unquote, open / close paren,
     "in parentheses", "in brackets" or "dash" today: 0/14 deterministic, and 0/14 with the
     LLM. The LLM is told nothing about them, and when it does format them, `guard::check`
     rejects the result as `llm_dropped`, because "quote"/"unquote" count as dropped content
     words. Prototype rules gated on pause commas (`rules_proto.py`) give quote 6/8,
     paren 4/6 and dash 1/1, with **0 new false positives** on 16 negatives. Overall pass goes
     59 % → 72 % at 0 ms.
2. **The LLM's real contribution is narrow: run-ons, fillers, some corrections.** With the
   improved prompt (`prompt_v2.json`) and a guard that ignores command words, the 1.5B model
   on top of rules + casual styles reaches **80 %** pass at a p50 of **1273 ms**.
   It also regresses things that the deterministic layer had right, notably turning
   "new line" text into lists. See §5 for 0.5B / 3B.
3. **Android with "Clean up with local AI" off gets no cleanup at all,** not even the
   deterministic part. `effectiveCleanup` sends `cleanup_level: none`, and the engine gates
   `spoken_cleanup`, layout and `close_sentence` on `cleanup_level != none`
   (`engine.rs:1376`, `engine.rs:1384`). That configuration scores **17 %**; the default
   deterministic pipeline scores 44 %.

Boundary design (ending a command without a close): §6. Recommendation, implementation split and D27 draft: §7.

## 1. Inventory: what we handle today

### 1.1 Deterministic layer (always on unless `cleanup_level` is none)

Order inside `Engine::finish` (`crates/flow-core/src/engine.rs:1358-1427`):

1. `spoken_cleanup` (`postprocess/spoken.rs:945`).
2. `layout` (`spoken.rs:1314`).
3. `close_sentence` (`postprocess/terminal.rs`, D25).
4. The optional LLM.
5. The client post-processing `postprocess::apply` (`postprocess/mod.rs:239`).

| Area | Handled | Where |
|---|---|---|
| Spoken punctuation | new paragraph, new line, question mark, exclamation mark/point, full stop, period, comma, semicolon/semi colon, colon. A noun guard skips the word after "a / the / Oxford / trial / grace …", and a sentence-initial command is skipped | `spoken.rs:79-91` (list), `:33-77` (noun guard), `:296-331` |
| **Not handled** | quote / unquote, open/close/end quote, open/close paren(thesis), in parentheses, in brackets, dash / hyphen / em dash, ellipsis, "dot" | (nothing) |
| Fillers | um/uh/er/hmm…, ", like," and ", you know," between commas, "like" before a number after a form of "be", a leading "so"/"okay so" | `spoken.rs:27`, `:333-414` |
| Stutters | "I I" → "I" (KEEP_DOUBLE exceptions). **Bug:** this also collapses repeated number words, so "five five five, one two three" becomes "five, one two three" (case i6) | `spoken.rs:424-441` |
| Backtracking | retract: scratch/strike/delete/forget/cancel that, never mind. Correct: actually I mean, sorry I mean, no wait, wait no, or rather. Literal guards ("don't forget that", "there's no wait") | `spoken.rs:95-134`, `:444-563` |
| ITN | money, percent, clock times with am/pm or "o'clock", "Month Nth" dates. **Not handled:** bare cardinals ("ten minutes", "two hundred and fifty"), a time with no am/pm ("seven thirty tonight"), "the twenty first of October", decimals ("two point three"), phone digit strings | `spoken.rs:713-905` |
| Lists | ordinal markers (first/second/third…, "number one…") → `1.` lines with an intro colon. **Not handled:** bare cardinal markers ("one, hiring, two, …"), unmarked enumerations | `spoken.rs:977-1220` |
| Email layout | greeting line, body, sign-off + name (category email). **Bugs:** "Dear Mr. Patel," is split at "Mr." (`GREETING` ends the name at any `.`); a bare "Thanks." sign-off without a name is not split off | `spoken.rs:1226-1312` |
| Closing period | adds "." to open dictations of ≥ 4 words | `terminal.rs` (D25) |
| Period drop / casing / spacing | Cursor context: drops the trailing "." by **style** (formal never; casual ≤ 10 sentences; very casual always; default/excited only in messaging apps ≤ 2 sentences); cursor-context casing and spacing; trailing space | `postprocess/context.rs:150-170` |

### 1.2 LLM prompt (`crates/flow-core/src/cleanup.rs`)

* **System prompt** (`cleanup.rs:143-152`, short "fewshot" variant from docs/model-benchmarks.md):
  * remove fillers;
  * keep only the correction after a self-correction;
  * spoken comma / period / question mark / new line / new paragraph → symbols;
  * numbers, money, times and dates as digits;
  * 3+ items → HTML list;
  * email layout;
  * fix capitalisation and punctuation;
  * keep the language and dictionary spellings;
  * never answer.

  It says **nothing about quotes, parentheses, brackets, dashes, colons or exclamation**, and
  nothing about tone.
* **Level line** (`cleanup.rs:131-138`). Light (the only level the shells send by default):
  "only these fixes; keep the speaker's word choice and sentence order".
* **Style hint** (`cleanup.rs:155-163`), sent as `STYLE:` in the user message:

  | Style | Hint |
  |---|---|
  | casual | sentence case, drop optional commas, final period optional |
  | very casual | all lowercase |
  | excited | "!" |
  | formal / default | nothing |

  A casual style exists in the core. **No shell ever sends it:**
  * Android `Styles()` defaults every bucket to `FORMAL` (`apps/flow-android/.../core/FlowCore.kt:172-176`), and `toSessionConfig` never sets styles (`data/DataLogic.kt:123-133`). There is no style UI.
  * Windows defaults all four buckets to `"formal"` (`apps/flow-windows/src/prefs.rs:143-149`, comment "03 §1.3: every Wispr bucket defaults to formal + light").
* **Examples**: 7 few-shot turns (`cleanup.rs:168-185`). None shows quotes, parentheses or
  casual chat.
* **Guard** (`guard.rs`, the default output check; the edit-distance cap only applies when a
  shell sets `divergence_threshold`):
  * support ≥ 0.75 and coverage ≥ 0.5 per clause;
  * the spoken-command words it ignores are only comma / period / question / mark /
    exclamation / point / colon / semicolon / new / line / paragraph / full / stop
    (`guard.rs:38-52`).

  So every correct "quote … unquote" → `"…"` edit loses two content words, and the clause is
  rejected as **`llm_dropped`**. The same happens to "open paren", "in parentheses" and
  "dash", and to ITN the deterministic pass missed: "two hundred and fifty" → "250" is
  dropped, and "twenty first" → "21st" is unsupported.

### 1.3 What the shells send

| | Android | Windows |
|---|---|---|
| cleanup level | `effectiveCleanup` (`DataLogic.kt:120-121`): LLM off → **none** + `raw_mode` (no deterministic cleanup either, see §0 item 3); LLM on → selected strength, None promoted to **Light** | Light (`prefs.rs:141`) |
| styles | all formal (`FlowCore.kt:172-176`), no UI | all formal (`prefs.rs:143-149`), settings UI exists |
| budget | `ANDROID_LLM_TIMEOUT_MS = 3000` (`DataLogic.kt:88`) | `llm_timeout_ms = 1500` (`prefs.rs:168`) |
| category | `AppCategorizer` → email / work_chat / personal_chat / other (`policy/Policy.kt:76-93`); never "document" | own classifier (`context/mod.rs:186`) |

The user turned LLM cleanup on for both devices on 2026-10-24
(`docs/findings/2026-10-23-server-llm-cleanup-cost.md`, update box). So the live phone path is
Light + all formal + 3 s: column "llm (shipped)" below.

## 2. What Wispr does (from our research notes only)

This section is summarised from `research/windows/findings/by-topic/03-ai-formatting-and-features.md`
(W03), `research/windows/findings/data/style-config.json` (SC),
`research/windows/findings/by-topic/11-postprocess-port-spec.md` (W11) and
`research/android/findings/by-topic/06-postprocess-android.md` (A06). No vendor prompt text is
reproduced. Wispr's dictation formatter prompt is not in either client.

* **All shaping happens on the server.**
  * The Android client inserts the server's text byte for byte (A06:14-47).
  * The Windows client skips its own dictionary, context and period-drop steps whenever the
    server formatted the text (W11:18-29).
  * The server's pipeline stages are transcribe → align → format → lowercase → Slack tagging →
    email signature. The format stage reports deletion/substitution counts and a divergence
    score (W03:17).
  * Backtracking, filler removal, lists and punctuation are server-side; the client only
    detects them afterwards, to show tips (W03:38-41).
  * The client-side fallback formatter is a fine-tuned Llama-3.1-8B with a separate email
    variant (W03:7-8, 179).

  In other words: Wispr's server runs an 8B-class, task-tuned model where we run a 1.5B
  general instruct model with a 3 s budget on 4 cores.
* **Styles** are formal / casual / very casual / excited (+ genz = very casual), per bucket
  (personal / work / email / other) (SC:55-113):

  | Style | What it does |
  |---|---|
  | formal | capitals and full punctuation |
  | casual | capitals, fewer optional commas, no final period |
  | very casual | all lowercase, keeps "?" |
  | excited | "!" endings |

  * Email always gets blank-line structure (greeting / body / sign-off).
  * **Wispr Android's personal bucket defaults to Casual**; "other" defaults to Formal
    (`research/android/findings/by-topic/03-ui-features-onboarding.md:35`). Windows defaults
    every bucket to formal (SC:155-159). The formality detector can override the style for one
    dictation in personal messengers, based on the user's own visible messages (W03:62).
  * Style formatting is English-only (SC:115).
* **Period drop rule:** the same the cursor-context step table we ported (SC:203-228, W11:35-41).
* **Cleanup levels:**
  * The UI levels are none / light / medium / high. The server receives a *shifted* strength:
    UI high → MEDIUM, medium → LIGHT, light → VERBATIM (SC:43-53).
  * Even "verbatim" on the server removes fillers and applies style, per the UI's own example.
  * So the LLM's "light" in our prompt is stricter than what Wispr calls light.
* **Quote / unquote, parentheses, brackets:** our notes contain **no** record of these spoken
  commands. The only quote hits are cursor-context closing-quote handling (W11:123-165). The
  user's live observation today is therefore our only evidence. It is consistent with a
  server-side LLM that is asked to honour spoken formatting commands generally.

## 3. Eval set

`tools/probes/prototypes/formatting_eval_2026_10_24/`:

* `gen_cases.py` → `cases.jsonl`: **88 cases** in 12 categories:

  | Category | Cases |
  |---|---|
  | quote | 8 |
  | paren | 6 (from the coordinator's example) |
  | spoken_punct | 9 |
  | newline | 5 |
  | correction | 7 |
  | itn | 8 |
  | list | 5 |
  | casual | 7 |
  | email | 5 |
  | filler | 6 |
  | runon | 6 |
  | negative | 16 |

  * The negatives use the command words literally: "the word quote", "a quote for the roof",
    "trial period", "a period is called a full stop", "Oxford comma", "put a comma after",
    "a new line of sneakers", "the question mark at the end", "I mean it", "Quote me on that",
    "in parentheses on the board", "put the area code in parentheses", "in brackets at the
    bottom", "get me a quote".
  * **Inputs** are Parakeet-style: punctuated and capitalised, with a spoken command usually
    wrapped in pause commas. The run-on cases are the unpunctuated lowercase output Parakeet
    gives on fast speech.
  * **Expected outputs** follow the target app:
    * WhatsApp / Slack → casual (no final period);
    * Gmail → formal email layout;
    * Docs / Keep (category `other`, as Android sends) → formal punctuation.
  * `accept` lists equivalent renderings (comma inside/outside a closing quote, "4" vs "four").
* `score.py`: three metrics plus latency.
  * *exact*: equal up to trailing whitespace; the Android insert adds a space.
  * *pass*: normalised match against expected or any accept (curly → straight quotes, dash
    variants, whitespace collapsed; case and line breaks kept).
  * *sim*: difflib similarity.
  * Per-category pass, llm_ms and commit_ms p50/p90, and fallback reasons.
* `runner/`: a probe binary (own Cargo workspace, flow-core unmodified, path dependency). It
  drives `FlowCore::from_engines` with a text "ASR", like `flow-cli clean`. Platform android,
  `vad` / `hallucination_guard` off (no real audio), session files in `sessions/`.
  * `--prompt` swaps only the system prompt and the few-shot turns; the product's per-dictation
    user message is kept.
  * `--tuned-guard` replays `Engine::finish` step by step, with one change: the guard sees
    input and output with formatting-command words removed. This simulates adding them to
    `guard::SPOKEN`.
  * One untimed warm-up clean runs first, because the product prefills the static prefix while
    the user speaks. Runs without it timed out the first 4 cases on the longer v2 prefix (kept
    in `results/coldstart/`).
* `rules_proto.py` → `cases_rules.jsonl`: the proposed deterministic quote / paren / bracket /
  dash rules, applied as a pre-pass. They are the Python stand-in for a `spoken.rs` change.
* `prompt_v2.json` / `prompt_v3.json`: improved system prompt + 14 / 16 few-shot turns.
  * v2 adds: spoken quote / paren / bracket / dash / colon / exclamation rules, literal-use
    guards, phone numbers, run-on punctuation, and "add nothing".
  * v3 adds: "keep existing line breaks/quotes/lists" plus two examples.
  * No example overlaps a test case.

Box: i7-6700 (4C/8T), no GPU, 20 GB RAM with swap full. flow-server with Qwen 1.5B was
resident throughout. LLM threads = 4, `nice`, greedy decoding (flow-llm plugin). Load average
during the runs is in `results/run_all.log`. Absolute latency is "shared, loaded box";
compare columns, not the numbers against other boxes.

## 4. Results

| category | n | off | det | det+styles | det+rules+styles | llm (shipped) | llm+styles | v2 | v2 tuned | rules+v2 tuned | rules+v3 tuned |
|---|---|---|---|---|---|---|---|---|---|---|---|
| quote | 8 | 0/8 | 0/8 | 0/8 | 6/8 | 0/8 | 0/8 | 0/8 | 5/8 | 6/8 | 7/8 |
| paren | 6 | 0/6 | 0/6 | 0/6 | 4/6 | 0/6 | 0/6 | 0/6 | 4/6 | 5/6 | 5/6 |
| spoken_punct | 9 | 0/9 | 8/9 | 8/9 | 9/9 | 7/9 | 7/9 | 7/9 | 7/9 | 8/9 | 8/9 |
| newline | 5 | 0/5 | 5/5 | 5/5 | 5/5 | 2/5 | 2/5 | 2/5 | 2/5 | 2/5 | 3/5 |
| correction | 7 | 0/7 | 3/7 | 5/7 | 5/7 | 3/7 | 5/7 | 6/7 | 6/7 | 6/7 | 5/7 |
| itn | 8 | 0/8 | 3/8 | 3/8 | 3/8 | 4/8 | 4/8 | 4/8 | 4/8 | 4/8 | 4/8 |
| list | 5 | 0/5 | 2/5 | 2/5 | 2/5 | 4/5 | 4/5 | 3/5 | 3/5 | 3/5 | 3/5 |
| casual | 7 | 1/7 | 1/7 | 7/7 | 7/7 | 1/7 | 7/7 | 7/7 | 7/7 | 7/7 | 7/7 |
| email | 5 | 1/5 | 3/5 | 3/5 | 3/5 | 3/5 | 3/5 | 4/5 | 4/5 | 4/5 | 4/5 |
| filler | 6 | 0/6 | 1/6 | 3/6 | 3/6 | 3/6 | 6/6 | 6/6 | 6/6 | 6/6 | 6/6 |
| runon | 6 | 0/6 | 0/6 | 0/6 | 0/6 | 3/6 | 4/6 | 4/6 | 4/6 | 4/6 | 4/6 |
| negative | 16 | 13/16 | 13/16 | 16/16 | 16/16 | 13/16 | 15/16 | 15/16 | 15/16 | 15/16 | 15/16 |
| **pass** | 88 | 17% | 44% | 59% | 72% | 49% | 65% | 66% | 76% | 80% | 81% |
| **exact** | 88 | 17% | 41% | 50% | 60% | 45% | 59% | 59% | 68% | 70% | 72% |
| **sim** | 88 | 0.841 | 0.930 | 0.932 | 0.965 | 0.935 | 0.939 | 0.944 | 0.966 | 0.975 | 0.975 |
| llm ms p50/p90 | | 0/0 | 0/0 | 0/0 | 0/0 | 1061/1619 | 1140/1684 | 1295/1893 | 1310/1951 | 1273/1816 | 1332/1975 |
| commit ms p50/p90 | | 0/0 | 0/0 | 0/0 | 0/0 | 1061/1637 | 1143/1684 | 1295/1893 | 1310/1951 | 1273/1819 | 1333/1975 |
| fallbacks | | - | - | - | - | llm_dropped 13, llm_unsupported 3 | llm_dropped 15, llm_unsupported 3 | llm_dropped 14, llm_unsupported 3 | llm_dropped 3, llm_unsupported 3, only_filler 1 | llm_dropped 2, llm_unsupported 3, only_filler 1 | llm_dropped 3, llm_unsupported 3, only_filler 1 |

Columns:

| Column | What runs |
|---|---|
| **off** | Android with local AI off: `cleanup_level: none`, so no deterministic cleanup at all |
| **det** | Today's deterministic pipeline, styles as shipped (all formal), no LLM |
| **det+styles** | Same, with personal/work chat = casual, email/other = formal |
| **det+rules+styles** | + the prototype quote/paren/dash rules |
| **llm (shipped)** | Qwen2.5-1.5B, current prompt, product guard, shipped styles, 3 s budget. *This is what the phone gets today with local AI on.* |
| **llm+styles** | Same, with casual chat styles |
| **v2** | Improved prompt, product guard |
| **v2 tuned** | Improved prompt, guard ignores command words, raw inputs (LLM-only command handling) |
| **rules+v2 tuned** | Rules pre-pass + improved prompt + tuned guard (the proposed stack) |
| **…v3** | Same with prompt v3 |
| **0.5B / 3B** | The proposed stack with Qwen2.5-0.5B / 3B (same quant) |
| **30 s** | The proposed stack with no practical budget (quality ceiling of the 1.5B) |

Reading the table:

* **The 0 ms stack (det + rules + styles, 72 %) beats every LLM column that lacks the rules.**
  The best LLM stack (rules + v2 + tuned guard, 80 %) adds +8 pp for **+1.27 s p50 /
  +1.8 s p90**.
* **Where those 8 pp come from:**

  | Category | 0 ms stack | Best LLM stack | Change |
  |---|---|---|---|
  | filler | 3/6 | 6/6 | +3 |
  | runon | 0/6 | 4/6 | +4 |
  | paren | 4/6 | 5/6 | +1 (p1, no pause commas) |
  | correction | 5/7 | 6/7 | +1 |
  | email | 3/5 | 4/5 | +1 |
  | list | 2/5 | 3/5 | +1 |
  | newline | 5/5 | 2/5 | −3 (turns lines into lists) |
  | spoken_punct | 9/9 | 8/9 | −1 |
  | negative | 16/16 | 15/16 | −1 |

* **The prompt rewrite alone gains nothing while the product guard stays** (v2 66 % vs
  llm+styles 65 %): 14 outputs are still rejected as `llm_dropped`. Guard tuning plus v2 is
  +11 pp.
* **v3** ("keep existing formatting") = v2 within noise: 81 % vs 80 %, newline +1, correction −1.
* **No 3 s timeouts on the 1.5B once the prefix is warm.** The 30 s run is identical to 3 s
  (80 %). The budget is not what limits the 1.5B here: the guard and the prompt are.
* **Cold start:** the v2 prefix (~2× the current one) needs **~14 s** of prefill on this box
  (warm-up line in `results/run_all.log`); the current prefix needs 0.5 s, because it is already
  prefilled at load. Without prefill-at-load for the new prefix, the first ~4 dictations after
  a restart time out (`results/coldstart/`).

### 4.1 Why the LLM fails where it fails

* **Guard rejections are the main reason the current LLM adds nothing on commands.** Before
  tuning, `llm (shipped)` logged 13 × `llm_dropped` and 3 × `llm_unsupported` in 88 cases
  (no timeouts once warm).
  * The dropped clauses are the quote / paren / dash cases (the model *did* convert some), the
    ITN cases ("two hundred and fifty" → "250") and corrections without a listed trigger
    ("three, sorry, four").
  * With command words ignored, the 1.5B with the old prompt still did quotes only partly:
    "He called it, unquote, innovative." and "Quote: \"…\"". With v2 it produced "(the USB-C
    one)" even without pause commas (p1, the coordinator's example).
* **The LLM is told "light: keep word choice", and it is small.** It turns "new line"-formatted
  text into bullet lists (n1, n4), changes "comma" to ":" (s7) and drops "that" from "Don't
  forget that" (x6). These are regressions against the deterministic layer, which had them
  right.
* **Some failures are deterministic bugs that the LLM can't repair, because the guard then
  rejects the repair:**
  * i6: the stutter pass ate "five five" before the LLM saw it;
  * e2: "Dear Mr," / "Dear Patel,";
  * f3 / f4: a dangling comma after filler removal ("wanted to, check in");
  * f2: "So, like," became "Like,".

## 5. Model size, budget and cloud

| category | n | 0.5B | 1.5B | 3B | 1.5B, 30 s budget |
|---|---|---|---|---|---|
| quote | 8 | 7/8 | 6/8 | 6/8 | 6/8 |
| paren | 6 | 4/6 | 5/6 | 3/6 | 5/6 |
| spoken_punct | 9 | 8/9 | 8/9 | 9/9 | 8/9 |
| newline | 5 | 4/5 | 2/5 | 2/5 | 2/5 |
| correction | 7 | 5/7 | 6/7 | 5/7 | 6/7 |
| itn | 8 | 4/8 | 4/8 | 5/8 | 4/8 |
| list | 5 | 3/5 | 3/5 | 3/5 | 3/5 |
| casual | 7 | 7/7 | 7/7 | 5/7 | 7/7 |
| email | 5 | 3/5 | 4/5 | 4/5 | 4/5 |
| filler | 6 | 4/6 | 6/6 | 4/6 | 6/6 |
| runon | 6 | 1/6 | 4/6 | 2/6 | 4/6 |
| negative | 16 | 15/16 | 15/16 | 12/16 | 15/16 |
| **pass** | 88 | 74% | 80% | 68% | 80% |
| **exact** | 88 | 62% | 70% | 59% | 70% |
| **sim** | 88 | 0.973 | 0.975 | 0.962 | 0.975 |
| llm ms p50/p90 | | 599/829 | 1273/1816 | 2371/3050 | 1274/1876 |
| commit ms p50/p90 | | 599/844 | 1273/1819 | 2371/3050 | 1274/1876 |
| fallbacks | | llm_dropped 3, llm_unsupported 2, only_filler 1 | llm_dropped 2, llm_unsupported 3, only_filler 1 | llm_timeout 14, llm_dropped 5, llm_unsupported 3, only_filler 1 | llm_dropped 2, llm_unsupported 3, only_filler 1 |

Model notes:

* **0.5B** (491 MB gguf, load 4 s, cold prefix 6.6 s): 74 % at **p50 599 ms / p90 829 ms**,
  about half the 1.5B latency. It loses on run-ons (1/6), fillers (4/6) and email (3/5).
  Its +2 pp over the 0 ms stack is not worth 0.6 s.
* **3B** (2.1 GB gguf, load 23 s, cold prefix 28 s): **worse** than the 1.5B at the 3 s budget
  (68 %). 9 of its failures are timeouts (14 timeouts overall), with p50 2371 ms / p90 3050 ms
  (the cap). Memory allowed it (5.9 GB available before loading; ~4 GB needed), but it does
  not fit this box's latency budget. A 3B at a 6-8 s budget was not run; that is outside any
  interactive target.
* **1.5B is the only local size where the LLM pays for itself,** and only on run-ons and
  fillers.

**Cloud LLM (opt-in, user's own key; not measured, no keys on the box).**

* *Latency estimate.* Published figures for hosted OpenAI-compatible endpoints put a short
  formatting call (~700 cached-prefix prompt tokens + ~40 output tokens) at roughly:
  * Groq-class LPU serving of 8B models: 0.2-0.5 s (time-to-first-token ~0.1-0.3 s,
    several hundred tok/s);
  * OpenRouter / OpenAI small models: 0.5-1.5 s (TTFT 0.3-0.7 s, ~50-150 tok/s).

  Add the client→provider round trip. These are estimates from vendors' public numbers, not
  measurements.
* *Quality.* 8B+ instruct or frontier-small models are the class Wispr itself uses (fine-tuned
  Llama-3.1-8B fallback, W03:7-8). They are expected to handle quote / paren boundaries and
  run-ons much better than 0.5-3B.
* *Privacy.* Only **text** leaves the device or server. Audio stays local, unlike D18 cloud
  STT, where audio leaves. It is still the full dictation plus the app category (and context
  lines if sent) going to a third party. It must be **opt-in per user**, with the D18
  pattern: off by default, explicit consent text, the user's own key, a per-provider host
  allowlist, no logging of the key, a budget cap, and the never-fail rule (timeout → local
  result). Sending `TEXT BEFORE/AFTER CURSOR` should be a separate opt-in, or omitted.

## 6. Ending a command without an explicit close: how the boundary is decided

"quote … unquote" and "open paren … close paren" are easy, because the span is between the
two words. The hard cases have no close:
* "in parentheses the USB-C one to the meeting";
* "he just said, quote, whatever";
* "quote unquote X".

1. **Pause commas from Parakeet. Available today; used by the prototype.** Parakeet TDT puts
   a comma where the speaker pauses. "…the charger, in parentheses, the USB-C one, to the
   meeting." gives a clean span, from the comma after the command to the next comma or
   sentence end.
   * Rule: an unpaired command fires **only** when it is comma-wrapped. That also is the
     negative-case guard: "the formula is in parentheses on the board", "put the area code in
     parentheses" and "Quote me on that" have no pause commas and are left alone.
   * "quote unquote X" quotes only up to the next pause, or one word.
   * Prototype result: quote 6/8, paren 4/6 (p1 without commas left alone), 0/16 negatives
     broken.
2. **ASR word timings. Not available today, but cheap to add.**
   * sherpa-onnx already returns per-token `timestamps` (and TDT `durations`) in
     `OfflineRecognizerResult` (sherpa-onnx 1.13.8 `offline_asr.rs:619-624`).
     `flow-asr-sherpa::transcribe_with` throws them away and returns only `r.text`
     (`crates/flow-asr-sherpa/src/lib.rs:349-351`).
   * `AsrOutput` has no word field (`crates/flow-core/src/asr.rs:120-126`).
   * D22 segment cuts (≥ 800 ms pauses) are known inside the engine, but only the joined text
     survives (`engine.rs:821`).
   * Whisper can produce token timestamps too; not plumbed.
   * Plumbing `AsrOutput.words: Option<Vec<Word{text,start_ms,end_ms}>>` would let the rule use
     a real gap (say ≥ 250 ms) when Parakeet did not write a comma, and would also help
     run-on sentence breaks.
   * Cost: decode time ≈ 0; a few hundred bytes per dictation. Server mode needs no protocol
     change, because it runs inside FlowCore.
3. **LLM judgement, for no pause and no close (p1).**
   * The 1.5B with prompt v2 got p1 right ("(the USB-C one) to the meeting"), but it also got
     q6 wrong ("It was fine", the quotes dropped).
   * The deterministic rule should therefore leave such text **unchanged**, not guess, and the
     LLM may resolve it. The guard should accept a command word turning into a bracket pair
     only when the words in between are kept.
4. **Never-guess fallback.** No pause, no close, LLM off or rejected → leave the words as
   spoken. A literal "in parentheses" is a visible, easy-to-fix miss. Wrongly bracketed text is
   a silent meaning change.

## 7. Recommendation: the layered fix, in order of gain per ms

| # | Fix | Cost | Measured gain (pass, 88 cases) |
|---|---|---|---|
| 1 | **Style defaults + style UI.** Android: personal/work chat → casual, email/other → formal; Windows: same defaults, or keep formal but expose the choice. Send `styles` on Android | 0 ms | 44 % → 59 % (casual 1/7 → 7/7, negatives 13 → 16, corrections +2, filler +2) |
| 2 | **Deterministic spoken-command rules** in `spoken.rs`: quote / unquote, open / close / end quote, quote unquote X, open / close paren(thesis), in parentheses, in brackets, dash. Paired = exact; unpaired only when pause-comma-wrapped; noun / literal guards like `PUNCT_NOUN_BEFORE` | ~0 ms | 59 % → 72 % (quote 0 → 6/8, paren 0 → 4/6, dash) |
| 3 | **Deterministic bug fixes found by the eval:** no stutter collapse on number words; GREETING must not end at an abbreviation ("Mr.", "Dr.", "Ms."); drop the comma left before a removed filler; ", like," at sentence start after "So,"; bare "Thanks." sign-off split; ITN for "H MM" + tonight/morning/in the evening, cardinals ≥ 10 before a unit/noun, decimals ("two point three"), "the Nth of Month", digit strings → phone format; cardinal list markers ("one, X, two, Y, three, Z") | ~0 ms | itn 3/8 → est. 6-7/8, email 3 → 5, list +1, filler +2 (not measured; the cases are in the eval) |
| 4 | **Guard tuning:** add command words (quote, unquote, open, close, end, paren, parenthesis, parentheses, brackets, dash) to `guard::SPOKEN`. Treat number words and their digit forms as equal in support/coverage ("two hundred and fifty" = "250"). Let the guard accept removal of the clause *before* "sorry," / "I mean" when the next words line up (same as the backtrack exemption) | 0 ms | `llm_dropped` 14 → 3 with prompt v2; 66 % → 76 % ("v2" vs "v2 tuned") |
| 5 | **Prompt v2:** quote / paren / bracket / dash rules, literal-use guard, phone numbers, run-on punctuation, "keep formatting already present"; few-shots for each. The static prefix grows ~2×, which needs the prefill-during-speech path (already in the core) or the first dictation after load times out | ~+0-100 ms warm, cold first call needs prefill | 65 % → 66 % with the product guard; with the guard tuned, 67 % → 76 % ("cur tuned" run, `results/llm15_current_tuned.jsonl`) |
| 6 | **Category-aware LLM use:** run the LLM only where it adds value (unpunctuated run-ons, ≥ N words, or a command word the rules left unresolved). Skip it for short, already-punctuated chat messages; most of those are fully handled by layers 1-3 | saves ~1.3 s p50 on most chat dictations | (design; the eval shows the LLM changes nothing useful on casual/negative/newline cases) |
| 7 | **Bigger model / budget / cloud:** see §5. Only an opt-in cloud 8B+ model plausibly closes the run-on and boundary-without-pause gap at < 1 s; local 3B is too slow on this box | — | — |

Biggest gain per ms: **layers 1 + 2 (+ 3)**. They are ~0 ms, deterministic and testable, and
they fix exactly what the user named: quote / unquote, parentheses, casual chat, periods.
Layers 4-5 make the LLM *able* to help when it runs. Layer 6 stops it costing 1-3 s on
dictations where it can't.

### 7.1 Implementation plan (parallel, by file ownership)

**A. `crates/flow-core` — spoken rules (owner 1).**
`postprocess/spoken.rs`:
* a new `spoken_commands` pass before `spoken_punctuation`, token-based like the existing passes:
  * paired quote / paren / bracket spans;
  * unpaired spans only between pause commas;
  * "quote unquote X";
  * dash between words with a noun guard ("a dash of");
  * comma/period inside the closing quote;
  * capitalise a quote that opens a sentence;
* add "exclamation point" to the list (already present), plus "dash", "hyphen" (only between
  two words), "ellipsis", "dot dot dot";
* the stutter fix for number words.

Tests:
* every eval quote / paren / negative case as a unit test;
* `clean_text_is_unchanged` extended with the 16 negatives.

**B. `crates/flow-core` — layout / ITN fixes (owner 2).** All in `spoken.rs`, a separate
region from A: `GREETING` abbreviations, bare sign-off, filler comma cleanup, the ITN gaps,
cardinal list markers. Coordinate on `tokenize` / `render` only, without changing their
signatures.

**C. `crates/flow-core` — guard + prompt (owner 3).**
* `guard.rs`: `SPOKEN` += command words; number-word ↔ digit equivalence; the "sorry," /
  "I mean" correction exemption.
* `cleanup.rs`: SYSTEM + EXAMPLES from `prompt_v2.json` (keep `prompt_follows_03_section7_template`
  and the guard-passes-examples test green; raise the 1500-char cap in that test).
* Optional: an "LLM worth running?" predicate (layer 6) next to `should_run`, with
  `CommitResult.fallback_reason = "llm_skipped_clean"` for telemetry.

**D. `crates/flow-asr-sherpa` + `crates/flow-core/src/asr.rs` (owner 4, optional, after A).**
* Keep sherpa token timestamps;
* `AsrOutput.words`;
* the engine passes a pause mask to `spoken_commands`; segment joins keep word timings.

**E. `crates/flow-llm`.** No change needed. The plugin ABI is untouched by A-D.

**F. `apps/flow-android` (owner 5).**
* `Styles` defaults: personal = CASUAL, work = CASUAL, email = FORMAL, other = FORMAL
  (Wispr Android's own personal default is Casual);
* `toSessionConfig` sends `styles`;
* a Settings "Style" page with 4 buckets;
* **`effectiveCleanup`: LLM off must still send `cleanup_level: light` with `raw_mode: true`,**
  or the core must split the two gates (see C/G). Otherwise local-AI-off users keep getting
  zero cleanup.

**G. `crates/flow-core/src/engine.rs` (owner 3, small).** Gate `spoken_cleanup` / `layout` /
`close_sentence` on `spoken_cleanup && !verbatim_level`, not on `cleanup_level != none`.
`raw_mode` already skips the LLM. This is a behaviour change for Windows "none", so it needs a
D-row decision: none = truly verbatim, or "no LLM". Wispr's own "none" still removed fillers
on the server.

**H. `apps/flow-windows` (owner 6).** Same style defaults (or keep formal, the documented Wispr
Windows default, but surface the setting). Raise `llm_timeout_ms` default 1500 → 3000, which
matches Android: the cost finding showed 1500 ms times out on nearly everything.

**I. Eval as a regression gate.** Promote the probe to a `flow-core` test that runs
`cases.jsonl` through the deterministic path, with a floor of the measured pass rates per
category. The LLM columns stay a manual bench.

### 7.2 Draft decision row

| D27 | Dictation formatting parity (2026-10-24, `docs/findings/2026-10-24-formatting-gap.md`) | (1) Chat buckets default to **casual** on both shells (personal + work; email/other formal). Android sends `styles` and gets a style page. (2) Spoken formatting commands are deterministic in `spoken.rs`: quote/unquote, open/close/end quote, quote unquote X, open/close paren(thesis), in parentheses, in brackets, dash. A paired command converts exactly. An unpaired command converts only when Parakeet's pause commas bound it (later: ASR word gaps ≥ 250 ms); otherwise the words stay as spoken. Literal-use guards as for punctuation words. (3) The guard ignores formatting-command words and treats number words = digits. (4) The LLM prompt gains the same command rules + few-shots and "keep formatting already present". (5) The deterministic layer runs whenever `spoken_cleanup` is on, independent of the LLM switch. (6) Cloud text cleanup is a separate opt-in, D18-style (text only, own key, off by default). | 88-case eval (`tools/probes/prototypes/formatting_eval_2026_10_24/`): today's deterministic pipeline 44 % and the shipped LLM path 49 % (p50 +1.06 s); casual styles alone 59 %, + rules 72 % at 0 ms; rules + prompt v2 + tuned guard on 1.5B 80 % (p50 1273 ms). The guard rejected 13/88 LLM outputs as `llm_dropped`, mostly correct quote/paren/ITN edits. 0/16 negatives broken by the rules |

## 8. Reproduce

```sh
cd tools/probes/prototypes/formatting_eval_2026_10_24
python3 gen_cases.py && python3 rules_proto.py cases.jsonl > cases_rules.jsonl
CARGO_TARGET_DIR=../../../../target/formatting-eval cargo build --release --manifest-path runner/Cargo.toml
B=../../../../target/formatting-eval/release/formatting-eval
$B --cases cases.jsonl --session sessions/shipped.json > results/det_shipped.jsonl   # etc.
MODELS=<gguf dir> FLOW_LLM_LIB=<libflow_llm.so> ./run_all_llm.sh > results/run_all.log
python3 score.py cases.jsonl det=results/det_shipped.jsonl llm=results/llm15_current_shipped.jsonl --fails llm
```

## 9. Update (branch `feat/fmt-rules`): layers 2 + 3 in flow-core

The spoken-command rules (§7 layer 2) and the deterministic bug fixes (layer 3) are now in
`crates/flow-core/src/postprocess/spoken.rs` and its child module `spoken_commands.rs`, inside
`spoken_cleanup` (same `spoken_cleanup` / `cleanup_level` gate as before). `run_det.sh` scores
the real pipeline on the raw cases (no `rules_proto.py` pre-pass) and fails below the D27 floor
(72 %, 16/16 negatives):

| category | n | det | det+styles | proto+styles (§4) | **core rules** | **core rules+styles** |
|---|---|---|---|---|---|---|
| quote | 8 | 0/8 | 0/8 | 6/8 | 7/8 | **8/8** |
| paren | 6 | 0/6 | 0/6 | 4/6 | 6/6 | **6/6** |
| spoken_punct | 9 | 8/9 | 8/9 | 9/9 | 9/9 | **9/9** |
| newline | 5 | 5/5 | 5/5 | 5/5 | 5/5 | **5/5** |
| correction | 7 | 3/7 | 5/7 | 5/7 | 3/7 | **5/7** |
| itn | 8 | 3/8 | 3/8 | 3/8 | 7/8 | **8/8** |
| list | 5 | 2/5 | 2/5 | 2/5 | 3/5 | **3/5** |
| casual | 7 | 1/7 | 7/7 | 7/7 | 1/7 | **7/7** |
| email | 5 | 3/5 | 3/5 | 3/5 | 5/5 | **5/5** |
| filler | 6 | 1/6 | 3/6 | 3/6 | 3/6 | **6/6** |
| runon | 6 | 0/6 | 0/6 | 0/6 | 0/6 | **0/6** |
| negative | 16 | 13/16 | 16/16 | 16/16 | 13/16 | **16/16** |
| **pass** | 88 | 44% | 59% | 72% | 70% | **89%** |
| **exact** | 88 | 41% | 50% | 60% | 58% | **74%** |

* *core rules* = shipped styles (all formal); its 3 negative misses are the formal period on
  chat cases, fixed by the casual styles (layer 1), not by rules.
* Decisions taken while implementing:
  * Spoken "dash" → spaced em dash `" — "`.
  * "quote unquote X" quotes one word, or up to the next pause when one follows.
  * The pause comma before an opening mark is dropped, except before a 2+-word quotation
    after a speech verb ("She said, "I'll be there soon," and …"). That form capitalises the
    quotation and keeps its closing comma.
  * Pause-less p1 ("the charger in parentheses the USB-C one to the meeting") is handled by
    one narrow shape: a determiner-led aside of 2-6 words after a plain noun, closed by a
    preposition. Everything else without pause commas or a closer stays as spoken.
* Remaining misses are the LLM's share: run-ons (6), unmarked lists l4/l5, and corrections
  without a listed trigger (c4 "three, sorry, four", c7 "main, I mean to").
