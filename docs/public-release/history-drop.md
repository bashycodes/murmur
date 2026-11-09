# History drop list (D82)

Paths to filter out of the full history before the public repo is cut.
They held a port of another product's client code or data derived from it.

Drop (every revision):

- `crates/flow-core/src/postprocess.rs` (pre-split path)
- `crates/flow-core/src/postprocess/{android,context,cursor_tag,dict,press_enter,slack,stacked,text}.rs`
- `crates/flow-core/src/postprocess/apps.rs` and `mod.rs`: revisions before D82
- `crates/flow-core/src/learn.rs`: revisions before D82
- `crates/flow-core/data/{a11y-lists-android,app-categories-android,app-lists-windows}.json`
- `crates/flow-core/tests/vectors.rs`, `crates/flow-core/tests/vectors/`
- `research/` (whole subtree; removed from the tree by public A1)
- `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/wispr_cases.jsonl`
- `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/gen_wispr_cases.py`
- `tools/probes/prototypes/wispr_ab_2026_10_25/`

Redact rather than drop:

- `crates/flow-core/src/postprocess/spoken.rs`: older revisions quoted the
  other product's prompt.
