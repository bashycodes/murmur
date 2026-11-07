# Capitals in the middle of sentences (2026-11-07)

Ticket: "words in the middle of sentences are often capitalised". Branch
`feat/random-capitals`, decision D70. Probe scripts:
`tools/probes/prototypes/random_capitals_2026_11_07/` (scripts only; the
owner's history, recordings, transcripts and every output stay in
`eval-private/`). Examples below are invented with the same shape.

## Data

* PC history (copy of `history.sqlite`, read-only) and the PC debug log
  (`dictations.jsonl`): 87 English dictations (hold/tap) from 0.9.1 to
  1.1.2, Parakeet unified (greedy/auto) on the PC, "Decode while speaking"
  on (`segment_pause_ms` 800), local Llama 3B cleanup that mostly did not
  run (3 s timeout), 2 rows on flow-server. 15 rows still have audio.
* Phone history: not available while this ran; not used.
* Parity main set (13 clips, cased Wispr references) and the parity
  English set (30 clips), Parakeet unified greedy, PC session, rerun with
  flow-cli from this branch: bridge joins (product), plain joins, whole
  clip.

A "wrong capital" is a capitalised word that is not at a sentence start and
is not a name: in history judged by hand, in the parity sets automatic
(the reference has the word only in lower case).

## Where the wrong capital first appears (PC history)

| Stage | Wrong capitals | Rows |
|---|---|---|
| raw (ASR text after segment joins) | 7 | 5 |
| AI cleanup output (2 of the 5 rows were cleaned) | 0 | 0 |
| final pasted | 4 | 3 |

By segments: dictations decoded in one piece 0 of 73 rows; decoded in 2+
pieces 5 of 14 rows. By version: 0.11.1 2, 0.12.0 4, 1.1.2 1 (both old and
recent). Nothing comes from post-processing, the dictionary, insertion
context or number rules: every wrong capital is already in `raw`, and
cleanup, when it ran, lower-cased them. No server row had one.

## Root causes (both at segment joins, `segment.rs`)

A piece decoded alone starts with a capital and Parakeet often ends it
without a stop. The bridge decode around each cut is supposed to fix the
words next to the cut (D22 splice), but:

1. **The splice is all or nothing.** When any word in its span differs
   between piece and bridge ("quote-unquote" against "quote unquote", a
   different short word two words from the cut), it refuses, and the
   right piece keeps its leading capital: "we should look at Options for
   the meeting room". Nothing lower-cased a capital after a word with no
   stop (`capitalize_joins` only capitalises after one).
2. **Mixed sources inside an accepted splice.** The two words at each
   bridge edge keep the piece's spelling; the word after them takes the
   bridge's. When the bridge heard a sentence break right at its edge
   ("fine. The") and the piece a comma ("fine, the"), the result was
   "fine, The".

Reproduced on real audio: parity main set, bridge joins, 5 wrong capitals
in 3 clips (whole-clip decode: 0); 2 of them are cause 1. One PC
recording reproduces cause 2 exactly. The parity English set's 2 wrong
capitals are in the whole-clip decode too (ASR, not joins).

## Fix (D70)

* `segment::respell_cut`: when the splice is refused, the two words at
  the cut take the bridge's case and punctuation, if the bridge has that
  pair (same normalised words) exactly once, clear of its edge words.
  Words never change, so names the bridge capitalises stay capitalised.
* In the splice, a word that follows a word without a sentence end and
  differs from the piece's spelling only by a capital first letter takes
  the piece's spelling back.

No cleanup-prompt change: cleanup already fixed these when it ran; the
capital came from a deterministic stage of ours.

## Before / after (same build otherwise, Parakeet unified greedy)

| Set | Wrong capitals before → after | Clips changed |
|---|---|---|
| Parity main (13) | 5 → 3 | 2 (case and punctuation at a cut only) |
| Parity English (30) | 2 → 2 | 0 |
| PC recordings (15) | 1 → 0 | 1 |

The 3 left in the main set are a three-word product name said in title
case. Word error rate is unchanged by construction (no word changes; the
outputs differ only in case and punctuation at cuts). Regression tests:
`segment::tests::respell_cut_takes_the_bridge_case_at_the_cut`,
`splice_keeps_case_next_to_the_bridge_edges` (the "fine, The" case) and
`tests/segments.rs::a_pause_mid_sentence_does_not_capitalise_the_next_piece`
(fails on the old code with "w5 W6").

## Not covered

* The 4 history rows without audio cannot be replayed; they match cause
  1's shape (a piece-initial capital after a word without a stop, all in
  multi-piece dictations).
* When the bridge itself hears a different word at the cut, no rule
  applies and the capital can stay.
* One history row had a spoken "new paragraph" at a cut left as words; not
  looked into here.
