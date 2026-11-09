# server_cleanup_speed_2026_11_07

Probe for D69 (ticket "Parity · Faster server cleanup on long clips"),
findings in `docs/findings/2026-11-07-server-cleanup-speed.md`. Dated: no
permanent tool path.

- `runner/`: a copy of `dictionary_2026_11_07`'s runner (package
  `server-speed-clean`) that also prints the plugin's `last` counters
  (prompt, prefill and generated tokens, prefill and generation ms) per case.
  Build: `cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml`.
- `run.sh` replays one case file in `server` mode; `matrix.sh` runs several
  variants for R interleaved rounds (`'TAG|FLOW_LLM_DRAFT=4|'`); env from a
  copy of `env.example.sh`.
- `live.sh` starts a local flow-server build on its own port, state dir and
  token, sends every clip through the parity eval's `run_paths.py server`,
  and stops it.
- `summary.py` (runner rows), `timings.py` (live rows), `rows.py` (either
  -> rows for the parity `score.py`), `same.py` (identical answers A vs B).

Cases, sessions, clips and every output stay in `eval-private/`.

The cache default is `~/.cache/flow-clone`; set CACHE=<dir> to reuse an existing cache.
