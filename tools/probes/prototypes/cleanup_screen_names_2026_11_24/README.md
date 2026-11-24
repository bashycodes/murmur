# cleanup_screen_names_2026_11_24

Probe for D108 (cleanup gets a filtered SCREEN NAMES line instead of every
screen word on its DICTIONARY line). Findings:
`docs/findings/2026-11-24-cleanup-screen-names.md`. Dated: no permanent tool
path.

- `prepare.py OUT.jsonl`: 40 cases from the invented English screens of
  `crates/murmur-core/fixtures/screen-names/chat-screens.json`: each `said`
  sentence with its planted name misspelled (hand-written in `HEARD`), on the
  screen as it is (`short`) and at the bottom of a 2000-character capture
  (`full`).
- `runner/` (`screen-names-clean`, own workspace, this checkout's
  murmur-core): replays the cases through `MurmurCore::commit` with the
  local llama.cpp model, as murmur-server does (rules, then prompt v2).
  `--arm new` is this checkout's prompt, `--arm old` rewrites it to the
  pre-D108 one (raw screen words on the DICTIONARY line), `--arm none` sends
  no screen words. Build:
  `CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml`.
- `run.sh OUT_DIR TAG [ARM...]`: one model, each arm in turn, under the build
  lock at low priority; env `CASES SESSION MODEL LIB BIN THREADS`.
- `session-server.json`: the server's budget (3000 ms + 15 ms per character,
  light level, cleanup never skipped). `session-generous.json`: 30 s, to see
  what a model spells when time is no limit.
- `score.py results/*.jsonl`: a case is right when every planted name is in
  the output exactly; Markdown table.
- `results/`: the case file and the rows of the runs in the findings note
  (all content invented).
