# 2026-11-07: dictionary words reach the AI cleanup (D64)

**Question.** On the parity main set, every AI path got 8/15 of the
named terms right and the two-word product name 0/4, although the owner's dictionary was
already in the session. Can the cleanup prompt fix near-miss spellings of
the user's dictionary entries without pre-AI replacement
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
   ("vora" / Vo-Ra), a near spelling (edit similarity >= 0.75), the same
   consonant skeleton at similarity >= 0.6 ("sip-sip" / ZipZip) or, for a
   word of 12+ letters, the same 7-letter opening. The window must start
   with the entry's sound (first skeleton letter), so "loom" does not name
   eLoom. Entries of up to 3 letters match only themselves, entries with
   under 2 letters or digits ("C++") are never named, and an entry already
   spelled right is not named. A dictation with nothing near gets the
   prompt without a dictionary.
2. **Prompt rules.** v2 (on-device, murmur-server) and v5 (D30 AI cleanup)
   say a word that sounds like an entry and is meant as that name takes the
   entry's spelling, and an ordinary word stays. The DICTIONARY line stays
   in the per-dictation user message, so the cached system prompt and
   few-shots are byte-identical with and without a dictionary (test
   `ai_cleanup_names_the_entry_in_the_user_message_and_keeps_the_cached_prefix`).
   The v2 and v5 system text itself changed, so the first dictation after
   the update rebuilds the cached prefix once (on the phone, the next
   prefill); after that it is reused as before.
3. **Skip gate.** On-device, a tidy line wakes the model when it holds a
   near spelling of an entry (`cleanup::dictionary_can_help`).
4. **Stutters.** The rules no longer collapse a repeated word whose repeat,
   joined or spaced, is near an entry (`terms::names_a_term`): "bop bop"
   for BopBop reaches prompt v2 whole instead of as "bop".
5. **One prompt builder.** `cleanup::session_prompt` builds every session
   cleanup prompt; the engine and murmur-replay's prompt override both call
   it, so a replay builds the prompt the product sends.

A first version also added a built-in list of 16 product names, behind a
"Known product names" switch. It was removed: four of the names were terms
the parity eval scores, and others were left out by hand to protect them,
so the list was tuned to the eval; the ticket also covered only the user's
own words. A built-in names list could be a future ticket, chosen by a
stated rule that does not depend on the eval (not built).

No post-recognition fuzzy replacer was built. The owner principle allows one
only as a fallback for paths with no AI cleanup and only if measured to help;
it was not built or measured here.

## Results

Main set (13 real clips, owner dictionary in the session; terms / WER /
format). "Before" = base commit with the same dictionary. Rows other than
AI 3B and murmur-server were measured with the built-in list still in; the
owner dictionary already holds the names the main set scores, and the
AI 3B rerun without the list matched (13/15, 3/4, 14.4 %, 25/31).

| Path | Model | Terms | Product name | WER | Format |
|---|---|---|---|---|---|
| AI cleanup | Qwen2.5 3B | 8 → 13/15 | 0 → 3/4 | 16.6 → 14.4 % | 24 → 25/31 |
| AI cleanup | Qwen2.5 1.5B | 8 → 9/15 | 0 → 0/4 | 13.1 → 12.4 % | 18 → 18/31 |
| murmur-server | 1.5B | 8, 8 → 9, 8, 8/15 | 0 → 0/4 | 11.5, 9.8 → 12.0, 10.7, 10.7 % | 19, 19 → 18, 19, 19/31 |
| phone (3 s budget) | 1.5B | 8 → 8/15 | 0 → 0/4 | 10.0 → 9.8 % | 18 → 18/31 |
| PC (3 s budget) | 3B | 10 → 10/15 | 0 → 0/4 | 9.6 → 9.6 % | 21 → 21/31 |

On the phone and PC most main clips are long and hit the 3 s LLM budget,
so the model's output is not used at all; the change cannot show there.
murmur-server lists every run: base twice, after three times (the last with
the final code). Its first after run's 18/31 and 12.0 % are noise: the
reruns give 19/31 and 10.7 %, and the base itself moved 11.5 → 9.8 %
between runs (several clips hit the 8 s budget, which varies with load).

Synthetic set (`tools/probes/prototypes/dictionary_2026_11_07/`, 34 made-up
lines: P user entries 12, B public product names not in the dictionary 8,
N look-alike negatives 12, K tidy 2). P, N and K are from the run with the
built-in list (it does not touch them); B is rerun without it, so B now
shows only what a model knows of those names unaided:

| Path | Before | After | P | B (with list) | N kept |
|---|---|---|---|---|---|
| AI 3B | 28/34 | 24/34 | 10 → 8/12 | 4 → 3/8 (7) | 12 → 11/12 |
| AI 1.5B | 23/34 | 24/34 | 6 → 6/12 | 3 → 4/8 (6) | 12/12 |
| murmur-server 1.5B | 21/34 | 24/34 | 4 → 7/12 | 3 → 3/8 (5) | 12/12 |
| phone 1.5B | 14/34 | 21/34 | 0 → 7/12 | 0 → 0/8 (5) | 12/12 |
| PC 3B | 14/34 | 20/34 | 0 → 7/12 | 0 → 0/8 (4) | 12 → 11/12 |

The 3B's P varies between runs (11/12 in an earlier run with the same
selection and rules; one row timed out at 30 s).

**Known cost:** on the 3B (AI path and PC), "Oscar's son" next to the entry
Oskarsson became "Oskar's son": a look-alike inside an ordinary phrase is
respelled toward the entry. The 1.5B kept it.

Tried and dropped: few-shots with a DICTIONARY line (phone 26 → 24/34), and
naming the heard words with each entry ("Talvik (heard "Talvic")"), which
made the 3B keep the heard spelling (P 11 → 3/12).

**Prompt length and latency** (synthetic set, dictionaries of 0 / 20 / 200
user entries padded with made-up names (measured with the built-in list,
which named a few names at 0 entries); mean prompt characters, LLM p50 on
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
  on the phone and murmur-server.
- Main set: format checks (lists, quotes, parentheses, new lines) 0 to +1
  per path, except murmur-server 19 → 18/31; WER down on every AI path except
  murmur-server (11.5 → 12.0 %).

## Gaps

- The 1.5B rarely rewrites a sound-alike that is itself a real word
  ("whisper" for an entry spelled otherwise); the 3B does. That is a model
  limit on the phone, murmur-server's default model and the 1.5B AI path.
- Product names that are not in the user's dictionary are left to what the
  model knows (B group above).
- A short misspelling such as "Daro" for Darrow is too far to be named.
- Long dictations hit the phone and PC 3 s budget, so their terms stay as
  heard.
</content>
</invoke>
