# 2026-11-07: dictionary words reach the AI cleanup, plus built-in product names (D64)

**Question.** On the Wispr parity main set, every AI path got 8/15 of the
named terms right and "Wispr Flow" 0/4, although the owner's dictionary was
already in the session. Can the cleanup prompt fix near-miss spellings of
dictionary entries and common product names without pre-AI replacement
rules (owner principle: text quality belongs in the prompt and the model),
and without touching ordinary look-alike words?

**Why the dictionary did nothing before.** Base code put every dictionary
word in the prompt's DICTIONARY line with no instruction on what to do with
it. On the phone and PC, D27's "already clean" skip also kept the model off
a tidy line that held a misspelled entry (synthetic set: the model ran on
2/34 lines).

## Change

1. **Selection** (`cleanup::terms`). A prompt names only the entries the
   transcript plausibly misspells, at most 20 (`MAX_PROMPT_TERMS`), closest
   first. A window of 1 to n+1 transcript words matches when, with case,
   spaces and punctuation ignored, it is the entry in another spelling
   ("wifi", "chat GPT"), a near spelling (edit similarity >= 0.75), the same
   consonant skeleton at similarity >= 0.6 ("tic-tic" / TickTick) or, for a
   word of 12+ letters, the same 7-letter opening. The window must start
   with the entry's sound (first skeleton letter), so "phone" does not name
   iPhone and "if I" does not name Wi-Fi. Entries of up to 3 letters match
   only themselves, and an entry already spelled right is not named. A
   dictation with nothing near gets the prompt without a dictionary.
2. **Built-in names** (`dictionary::BUILTIN_TERMS`, 16 public product
   names such as TickTick, Wispr Flow, WhatsApp, GitHub, Wi-Fi). They are
   added after the user's entries for the prompt only (never ASR hotwords,
   never the replacement matcher), with a "Known product names" switch on
   both apps (default on; `SessionConfig.builtin_terms`).
3. **Prompt rules.** v2 (on-device, flow-server) and v5 (D30 AI cleanup)
   say a word that sounds like an entry and is meant as that name takes the
   entry's spelling, and an ordinary word stays. The DICTIONARY line stays
   in the per-dictation user message, so the cached system prompt and
   few-shots are byte-identical with and without a dictionary (test
   `ai_cleanup_names_the_entry_in_the_user_message_and_keeps_the_cached_prefix`).
4. **Skip gate.** On-device, a tidy line wakes the model when it holds a
   near spelling of an entry (`cleanup::dictionary_can_help`).

No post-recognition fuzzy replacer was built. The owner principle allows one
only as a fallback for paths with no AI cleanup and only if measured to help;
it was not built or measured here.

## Results

Main set (13 real clips, owner dictionary in the session; terms / WER /
format). "Before" = base commit with the same dictionary.

| Path | Model | Terms | Wispr Flow | WER | Format |
|---|---|---|---|---|---|
| AI cleanup | Qwen2.5 3B | 8 → 13/15 | 0 → 3/4 | 16.6 → 14.4 % | 24 → 25/31 |
| AI cleanup | Qwen2.5 1.5B | 8 → 9/15 | 0 → 0/4 | 13.1 → 12.4 % | 18 → 18/31 |
| flow-server | 1.5B | 8 → 9/15 | 0 → 0/4 | 11.5 → 12.0 % | 19 → 18/31 |
| phone (3 s budget) | 1.5B | 8 → 8/15 | 0 → 0/4 | 10.0 → 9.8 % | 18 → 18/31 |
| PC (3 s budget) | 3B | 10 → 10/15 | 0 → 0/4 | 9.6 → 9.6 % | 21 → 21/31 |

On the phone and PC most main clips are long and hit the 3 s LLM budget,
so the model's output is not used at all; the change cannot show there.

Synthetic set (`tools/probes/prototypes/dictionary_2026_11_07/`, 34 made-up
lines: P user entries 12, B built-in names 8, N look-alike negatives 12,
K tidy 2):

| Path | Before | After | Negatives kept |
|---|---|---|---|
| AI 3B | 28/34 | 28–31/34 | 11/12 (12 before) |
| AI 1.5B | 23/34 | 26/34 | 12/12 |
| flow-server 1.5B | 21/34 | 26/34 | 12/12 |
| phone 1.5B | 14/34 | 26/34 | 12/12 |
| PC 3B | 14/34 | 24/34 | 11/12 (12 before) |

The one lost negative: "Oscar's son" next to the entry Oskarsson became
"Oskar's son" on the 3B.

Tried and dropped: few-shots with a DICTIONARY line (phone 26 → 24/34), and
naming the heard words with each entry ("Talvik (heard "Talvic")"), which
made the 3B keep the heard spelling (P 11 → 3/12).

**Prompt length and latency** (synthetic set, dictionaries of 0 / 20 / 200
user entries padded with made-up names (the after rows still name built-in
names at 0); mean prompt characters, LLM p50 on
the rows where the model ran; the machine was shared and loaded, so
latencies are noisy):

| Entries | Base AI 1.5B | After AI 1.5B | Base phone | After phone |
|---|---|---|---|---|
| 0 | 11936 ch, 1.8 s | 12354 ch, 1.7 s | 6102 ch, 1.3 s | 6213 ch, 1.5 s |
| 20 | 12139 ch, 1.5 s | 12363 ch, 3.4 s * | 6305 ch, 1.3 s | 6218 ch, 1.8 s |
| 200 | 13826 ch, every call fails | 12364 ch, 2.8 s * | 7992 ch, 2.2 s | 6218 ch, 2.4 s |

With 200 entries the base prompt is 4109 tokens, over the 4096-token
context, so AI cleanup failed on every dictation ("prompt too long"). After
the change the prompt length does not grow with the dictionary: it holds
only the matched entries (at most 20). The base phone column ran the model
on 2/34 lines and the after column on 10–24/34, so their timings cover
different lines. (*) p95 hit the 20 s timeout on a few rows under load.

The cached prefix is unchanged: system prompt and few-shots are the same
bytes with and without a dictionary (unit test), and the extra cost of a
dictionary is only the user-turn DICTIONARY line (about 10–40 characters
per named entry).

**Regressions.** The 12-entry made-up dictionary in the session:

- Lists and corrections (`lists_corrections_2026_11_06`, 39 cases, AI
  1.5B): base 29/39 and 26/39, after 26/39 and 28/39 over two runs each;
  every difference in lists was a 20 s timeout under load (base second run
  4 timeouts, after first run 3). Corrections 2/9 and negatives 15/16 in
  every run. AI 3B after: 30/39 (lists 8/12, corrections 8/9, negatives
  14/16); its base run did not finish within the run limit.
- Numbers (`numbers_2026_11_06`, 61 cases): 67/67 checks before and after
  on the phone and flow-server.
- Main set: format checks (lists, quotes, parentheses, new lines) 0 to +1
  per path, except flow-server 19 → 18/31; WER down on every AI path except
  flow-server (11.5 → 12.0 %).

## Gaps

- The 1.5B rarely rewrites "Whisper Flow" (a real English word plus one
  letter) to Wispr Flow; the 3B does. That is a model limit on the phone,
  flow-server's default model and the 1.5B AI path.
- The rules' stutter removal turns "tick tick" into "tick" before prompt v2
  sees it, so TickTick is not named on the on-device and server paths.
- A short misspelling such as "Daro" for Darrow is too far to be named.
- Long dictations hit the phone and PC 3 s budget, so their terms stay as
  heard.
</content>
</invoke>
