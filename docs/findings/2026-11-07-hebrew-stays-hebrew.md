# 2026-11-07: on-device cleanup translates long Hebrew dictations (D65)

**Question.** The on-device AI cleanup (fallback rules, then prompt v2 on
Qwen2.5 1.5B) returned English for long Hebrew dictations: 6 of 6 cleaned
synthetic Hebrew lines in the spoken-numbers probe
(`2026-11-06-spoken-numbers.md`), before and after D63. Why, and can the
prompt or the model keep them Hebrew without a pre-AI rule?

All Hebrew text here and in the probe is made up.

## Root cause

The model copies the language of its few-shots. Prompt v2 has an English
system prompt and 21 English few-shots, and nothing Hebrew. The language
hint is passed: a Hebrew dictation's user message carries
`LANGUAGE: Hebrew (write the text in this language; never translate)`
(D33), and the system prompt says "Keep the speaker's language". The 1.5B
does not hold either line against 21 English turns. Without the hint it
translated 18/22 long lines, with it 16/22 (table below). Nothing after the
model looked at the script: D59 removed every output check, so the
translation was committed.

The probe also shows why the owner sees English rather than a timeout. A
long Hebrew answer costs the 1.5B about 4-6 s on this shared 4-core box
(Hebrew takes many more tokens than its English translation), so with the
phone session's 3 s budget nearly every line timed out here and kept the
rules text. The likely reason the phone shows English: its faster CPU
finishes the short English translation inside the budget (not measured on
the phone).

## Fix

1. **A Hebrew few-shot, for Hebrew dictations only** (`cleanup::HEBREW_EXAMPLE`).
   A made-up casual chat line with a filler, its cleaned form, and the
   `LANGUAGE: Hebrew` line as a real Hebrew prompt has it. Prompt v2 appends
   it after the English few-shots when the dictation is Hebrew. The ASR
   language hint decides when there is one (`he`/`iw` adds it, any other
   language removes it, so a Yiddish dictation in Hebrew letters gets no
   `LANGUAGE: Hebrew` example); only without a hint (missing or `auto`) do
   the transcript's letters decide (mostly Hebrew, `script_lock::dominant`).
   murmur-server never sees an English hint (D33 sends only non-English), so
   there the letters decide for English too. Because it goes last, the cached prefix
   (system prompt and the 21 English turns, D63) is the same bytes for every
   language. An English prompt is unchanged byte for byte (unit test), and a
   Hebrew one re-uses that prefix and prefills only the Hebrew turn and its
   own message. v5 (cloud AI cleanup) is unchanged. The few-shot shares no 5-word
   run with any probe line.
2. **A script check after the model** (`cleanup::run_metered`, every cleanup
   path). When the transcript is in a non-Latin script (Hebrew, Arabic,
   Cyrillic, Greek) and the answer's share of that script is under half the
   transcript's own share (`script_lock::drifted_from`), the answer is not
   used. The dictation keeps the rules text, with
   `fallback_reason: llm_language_changed` (on the AI-cleanup path also the
   fallback detail "answer not in Hebrew"). Command Mode's lock
   (`script_lock::drifted`, 2026-11-01) uses an absolute 0.5; cleanup uses
   the ratio because a mixed dictation can be correctly cleaned below half:
   "אממ תעלה את ה release notes לדרייב ותשלח בסלאק לפני ה standup" is 62 %
   Hebrew letters, and its cleanup "תעלה release notes לדרייב ותשלח ב-Slack
   לפני standup." 45 %. The ratio keeps it (0.73), and still catches a
   half-translated answer (11 %, ratio 0.18). In the probe runs the two
   groups are far apart: all 68 translated answers at 0 % Hebrew, all 170
   Hebrew answers at ≥ 91 %. A dictation under 60 % Hebrew is never
   checked (unit tests at both edges). It does not judge the words, only
   the writing system, so English names inside Hebrew ("ה-PDF בזום") pass.
   A Latin-script dictation is never checked: English and Spanish can't be
   told apart by their letters.

No rule rewrites text before the model.

## Measurements

The real `MurmurCore` commit path (the probe runner, local mode = phone
pipeline, phone session: light, casual styles), Qwen2.5-1.5B Q4_K_M, on
this shared box. "Before" = the runner built against integration `12fa4ad`.
`20 s` rows give the model 20 s, so they show its language and not the
box's speed. Probe: `tools/probes/prototypes/hebrew_stays_hebrew_2026_11_07/`.
The runs are greedy: a re-run of the before row gave the same 28 answers.

**Choosing the change** (22 long lines of `he_cases.jsonl`, 20 s, prompt
variants on the base runner):

| variant | answer in Hebrew | words of the input kept | lines that lost > 20 % | LLM p50 / p95 |
|---|---|---|---|---|
| v2 as shipped (`LANGUAGE:` line) | 6/22 | 0.96 | 0 | 4.1 / 5.1 s |
| no `LANGUAGE:` line | 4/22 | 0.98 | 0 | 4.2 / 9.1 s |
| + a line after the transcript ("Write the text in Hebrew, the language of the TRANSCRIPT. Do not translate it into English.") | 17/22 | 0.99 | 0 | 4.8 / 5.3 s |
| + the Hebrew few-shot | **22/22** | 0.96 | 1 | 4.5 / 5.1 s |
| + both | 22/22 | 0.95 | 2 | 5.1 / 5.4 s |

**Held out** (14 more long lines, `he_holdout.jsonl`, written after the
variant was picked):

| variant | answer in Hebrew | words kept | lost > 20 % | LLM p50 / p95 |
|---|---|---|---|---|
| v2 as shipped | 4/14 | 1.00 | 0 | 4.7 / 5.1 s |
| + the line | 11/14 | 1.00 | 0 | 5.2 / 5.7 s |
| + the few-shot | **14/14** | 0.97 | 0 | 5.2 / 5.7 s |
| + both | 14/14 | 1.00 | 0 | 5.5 / 6.0 s |
| + few-shot and "Keep the text in Hebrew." | 14/14 | 0.99 | 0 | 5.1 / 5.8 s |

The few-shot alone does the job on all 36 lines. The extra line adds about
0.3-0.5 s and did not keep more lines Hebrew, so it was dropped.

**Shipped** (this branch's runner, no overrides):

| set | before: answer Hebrew / final Hebrew | after: answer Hebrew / final Hebrew | LLM p50 before → after |
|---|---|---|---|
| 22 long lines, 20 s | 6/22 / 6/22 | **22/22 / 22/22** | 4.1 → 4.6 s |
| 14 held out, 20 s | 4/14 / 4/14 | **14/14 / 14/14** | 4.7 → 5.3 s |
| 22 long lines, phone session (3 s) | 0/22 / 22/22 (all timed out) | 0/22 / 22/22 (all timed out) | 3.0 → 3.0 s |
| check alone: this branch, base prompt (no Hebrew turn), 20 s | | 6/22 / **22/22** (16 `llm_language_changed`) | |

The 6 medium and short lines (M, S) keep D27's "already clean" skip before
and after. With the few-shot, the cleaned lines are the dictation with
commas, sentence breaks and a final period, and the words kept stay at
0.97-0.98. The 1.5B still makes Hebrew word errors of its own: on one line
it wrote a Hebrew number word as "nineteen" inside the Hebrew sentence (the
check passes it: the line is still Hebrew).

**English** (the spoken-numbers probe's 61 English cases,
`numbers_2026_11_06/cases.jsonl`, scored with its `score_cases.py`):

| run | checks | cases | LLM ran | final text same as before | model answer same as before | LLM p50 / p95 |
|---|---|---|---|---|---|---|
| before, 20 s | 67/67 | 58/58 | 27/61 | | | 2.4 / 3.2 s |
| after, 20 s | 67/67 | 58/58 | 27/61 | 61/61 | 61/61 | 1.3 / 1.6 s |
| before, phone session (3 s) | 67/67 | 58/58 | 27/61 | | | 1.3 / 1.6 s |
| after, phone session (3 s) | 67/67 | 58/58 | 27/61 | 61/61 | | 1.3 / 1.6 s |

Every English line is the same text before and after. That is by
construction: an English prompt is the same bytes, and the script check
never runs on a Latin-script dictation. The 20 s "before" row ran while the
box was busier (load 7.9 vs 6.8); its time difference is the box, not the
change.

## Not done / open

What was measured: prompt v2 on Qwen2.5-1.5B (the phone's on-device model),
on this box, with a 20 s budget. What was not: the phone's 3 s session
budget, murmur-server's model, and prompt v5 (cloud AI cleanup).

* **Open question (needs a phone measurement after merge):** on the phone,
  does a long Hebrew line finish within the 3 s budget, or does it always
  time out and fall back to the rules text? If it always falls back, a
  long Hebrew dictation on the phone is pasted as the (Hebrew) rules text,
  and neither the few-shot nor the script check changes what is pasted.

* **Hebrew lines are slow on the 1.5B.** 4-6 s on this box, over the phone
  session's 3 s budget; the few-shot makes Hebrew answers slightly longer to
  produce (+0.5 s p50) and adds one Hebrew turn to prefill when a Hebrew
  dictation follows an English one. Not measured on the phone: there a
  timed-out Hebrew line keeps the (Hebrew) rules text.
* Murmur-server's cleanup builds the same v2 prompt from the client's
  request, so a Hebrew dictation cleaned there gets the few-shot too, and
  the client's core applies the script check to the server's answer. Not
  measured with the server's model.
* AI cleanup (v5, cloud / custom server) gets the script check but not the
  few-shot. Not measured; the check only changes a result when the model
  translated.
* No few-shot for Arabic, Cyrillic or Greek dictations (no ASR route for them
  yet, D33): they get the script check only.
* Measured on this box, not on the phone.
