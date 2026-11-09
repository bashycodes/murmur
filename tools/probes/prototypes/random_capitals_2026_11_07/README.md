# random_capitals (2026-11-07)

Probe for the ticket "words mid-sentence capitalised". Findings:
`docs/findings/2026-11-07-random-capitals.md` (D70).

Scripts only. The owner's history, recordings, references and every output
stay outside the repo; all paths are arguments. `--show` prints owner text:
never paste it into the repo.

| File | What |
|---|---|
| `history_midcaps.py DB [--show]` | mid-sentence capitals per stage (`raw`, `formatted`, `pasted`) in a `history.sqlite` copy |
| `jsonl_keys.py LOG` | row kinds and keys of a `dictations.jsonl` debug log |
| `jsonl_midcaps.py LOG [--show] [--words FILE]` | same per debug-log row; with a hand-judged `[ts, word]` list, counts by version, segments, cleanup, route |
| `export_wavs.py DB OUT` | the history's kept recordings as `OUT/audio/<id>.wav` |
| `run_set.py MURMUR_CLI WAV_DIR OUT.jsonl -- ARGS` | `murmur-cli transcribe --json` per WAV |
| `matrix.sh MURMUR_CLI PARAKEET_DIR SEG.json NOSEG.json OUT NAME=WAVS...` | bridge / plain / whole runs (`MODES` env), low priority |
| `capscore.py REFS.json OUT.jsonl [FIELD]`, `score_runs.sh REFS RUN_DIR` | wrong capitals against cased references |
| `diff_runs.py BEFORE AFTER [--show]` | clips whose text changed |
