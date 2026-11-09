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
- Old top-level paths from before `research/` existed:
  `findings/by-topic/06-postprocess-android.md`,
  `findings/by-topic/11-postprocess-port-spec.md`,
  `findings/data/postprocess-vectors.json`,
  `findings/data/postprocess-vectors-android.json`,
  `tools/probes/postprocess_android_ref.py`,
  `tools/probes/postprocess_cases.cjs`,
  `tools/probes/postprocess_harness.cjs`

Redact rather than drop:

- `crates/flow-core/src/postprocess/spoken.rs`: older revisions quoted the
  other product's prompt.
- `crates/flow-core/src/postprocess/terminal.rs` (pre-D82): the module
  comment described the other product's client post-processing functions
  and its ground-truth vectors.
- `apps/flow-windows/src/learn.rs` and
  `apps/flow-android/app/src/main/java/dev/flowclone/learn/EditWatch.kt`:
  older revisions said the end reasons mirror the other product's
  end-reason field.
- `apps/flow-android/app/src/main/java/dev/flowclone/learn/Suggestions.kt`:
  older revisions described how the other product adds learned words.
- `apps/flow-windows/src/context/mod.rs` and `context/uia.rs`: older
  revisions cited the other product's app lists and helper notes.
- `tools/probes/prototypes/cleanroom_2026_11_09/{gen_cases.py,cases.json,before.jsonl,after.jsonl}`:
  revisions before the neutral app id used the other product's exe name.

Found with `git log --all --format= --name-only | sort -u` and
`git log -G 'Wispr|M:[0-9]{4,}' --all --name-only`, limited to the text
post-processing and learning files. The other hits there are the old
project name in examples (dictionary entries, test strings); the rename
handles those.
