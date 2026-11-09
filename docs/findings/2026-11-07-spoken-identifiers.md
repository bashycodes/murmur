# 2026-11-07: spoken identifiers; digit-aware dictionary naming (D67)

**Question.** On the parity English set, every AI path missed both
identifier lines. One holds an environment-variable name that the speaker
says as plain words, with no spoken form cue ("in caps", "underscore"):
the speaker only says that it is a variable. The other holds an
architecture name with a number, which is in the user's dictionary. Can
the cleanup fix these generically (owner principle: no pre-AI rules, no
lookup table, nothing named after the eval's examples), without changing
prose that uses the same words?

**Answer.** Only the dictionary half. Shipped: D64's term selection now
names a dictionary entry with digits when the number is said as words.
Rejected: a "Code names" prompt rule with few-shots. It gained on the tuned
half of a synthetic set, but the held-out half stayed flat.

## Shipped: digit-aware dictionary naming (`cleanup::terms`, amends D64)

- **Before.** D64 compared windows of transcript words with each entry
  through spelling and a consonant skeleton. The skeleton drops digits,
  and the transcript says the number as words, so an entry like "Velk42"
  was never named for "velk forty two".
- **Digit copy of the transcript.** Selection now also matches against a
  copy of the transcript in which each run of number words is merged into
  one digit word:
  - "velk forty two" -> "velk 42";
  - adjacent numbers join, as when a code is read out: "three twelve" -> "312".
- **Scope.** Only entries that contain a digit use that copy.
- **Same digits required.** Any such entry needs exactly its digits in the
  window: "velk forty three" does not name Velk42, and "three twelve"
  names Rho312 but not Rho31 or Rho3.
- **Known risk.** An entry made of an ordinary word and a number is named
  in everyday prose that has both ("win ten dollars" names Win10). This was
  already the case under D64 whenever the speech model wrote the digits
  ("win 10 dollars"). Naming only puts the entry on the DICTIONARY line;
  the prompt keeps a word used in its ordinary sense, so the model decides.
  The tests record this limit.

**Results** (dictionary change only; base -> after; dictionary in the
session):

| path | identifier lines (6) | synthetic negatives (12) | English set WER, terms (37) | main set WER, terms (15) | synthetic LLM p50 |
|---|---|---|---|---|---|
| phone (v2, 1.5B) | 0 -> 3 | 11 -> 11 | 9.0 -> 8.7 %, 27 -> 28 | 10.2 -> 9.8 %, 8 -> 8 | 1365 -> 1357 ms |
| murmur-server (v2, 1.5B) | 0 -> 3 | 11 -> 11 | 9.3 -> 9.0 %, 27 -> 28 | 10.5 -> 9.8 %, 8 -> 8 | 1368 -> 1324 ms |
| AI cleanup (v5, 1.5B) | 0 -> 3 | 12 -> 12 | 8.7 -> 8.3 %, 29 -> 30 | 12.2 -> 12.2 %, 8 -> 8 | 1623 -> 1778 ms |
| AI cleanup (v5, 3B) | 0 -> 3 | 12 -> 12 | 6.6 -> 8.0 %, 30 -> 29 | 15.7 -> 15.5 %, 8 -> 8 | 2981 -> 3102 ms |

- **Identifier lines** (2 lines x 3 speech-model transcripts). The 3
  passes are the architecture name, on every path. The env-var line stays
  0/3. Without a dictionary entry, no path wrote the architecture name in
  its compact form.
- **3B English set.** The worse WER and the lost term come from two clips
  that hit the 30 s time limit on a heavily loaded machine (raw text
  pasted). The other 27 outputs are byte-identical to base. The main
  set's 3B also had one timeout.
- **Negatives.** The phone and murmur-server negatives miss the same case as
  base; no output changed.
- **Synthetic set.** Totals moved by at most 2 cases per path (variance;
  the prompt is the same).
- **Prompt size.** Unchanged: no rule or shot is added, and the DICTIONARY
  line gains the entry only when it is named. Latency differences are
  noise on the loaded machine.

## Rejected: a "Code names" prompt rule

**What was tried.** A generic prompt rule plus invented few-shots:

- The rule: a name said with its form is written in that form; an
  env-var or constant name is caps with underscores; a technical name with
  a number takes its written form.
- 7 shots on v2 and 2 on v5. 7 shots on v5 overflowed the local engine's
  4096-token context (4088 tokens), so every AI call failed with "prompt
  too long".
- Skip-gate cue words.

It was measured on the synthetic set in
`tools/probes/prototypes/spoken_identifiers_2026_11_07/`: 38 invented
cases, even-numbered half held out. Rule plus dictionary change, base ->
after:

| path | total (38) | tuned half (19) | held half (19) |
|---|---|---|---|
| phone (v2, 1.5B) | 21 -> 26 | 8 -> 14 | 13 -> 12 |
| murmur-server (v2, 1.5B) | 23 -> 28 | 9 -> 15 | 14 -> 13 |
| AI cleanup (v5, 1.5B) | 20 -> 25 | 8 -> 13 | 12 -> 12 |
| AI cleanup (v5, 3B) | 23 -> 25 | 11 -> 12 | 12 -> 13 |

On the parity data, with the rule:

- **Identifier lines.** 3-4/6 per path. That is at most one more than the
  dictionary change alone (3/6 on every path), from the env-var line on
  two paths.
- **English set WER.** Lower on every path (3B 6.6 -> 4.5 %).

**Regressions.**

- Spoken-numbers checks on the phone: 67 -> 66/67.
- Lists and corrections on AI 1.5B: 29 -> 28/39.
- The 3B wrote a prose "snake case" as SnakeCase.

**Cost.** The prompt grows by about 530 tokens on v2 and 280 on v5. LLM
p50 on AI 1.5B went from 1623 to 2054 ms (other paths +30 to +80 ms, on a
loaded machine).

**Why rejected.**

- **Owner rule.** A generic approach has to work well on the small
  models, and here the held-out half was flat, so the gain came from the
  tuned half.
- **The env-var part was eval-shaped.** The eval's env-var line has no
  form cue, so only the "env-var or constant is caps with underscores"
  clause could reach it. The speech model also damaged the word that marks
  the name as a variable in 2 of 3 transcripts, and the line still passed
  only 1/3.
- **Prose regression.** It cost one prose negative on the 3B.
- **Dash names.** It cannot help dash-joined names on v2, because the
  pre-AI rules already turn a spoken "dash" into " — ".
