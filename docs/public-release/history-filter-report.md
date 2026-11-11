# History filter report (public B)

> **Since 2026-11-11:** the first publish (Step C below) ran on 2026-11-09
> (public `main` `fad649b`). The filter now lives in `tools/public/`, and later
> updates go through the fast-forward-only sync in [sync.md](sync.md). The
> prototype paths below are the frozen audit trail.

For owner sign-off before anything is pushed. Nothing has been pushed; the
rewritten repo exists only in a local scratch mirror with no remote.

- Source: this branch's tip, filtered as `integration` (`SOURCE_REF=feat/public-history-filter run.sh`):
  `origin/integration` `8bb7e62` (A5 rename merged) plus this branch's own commits (the tool,
  this report and the HEAD-alignment commit), plus the 18 tags.
- Run: `~/.cache/murmur-filter/work/20261109T220339Z/` (`orig/` untouched, `repo/` rewritten).
- Tool: `tools/probes/prototypes/history_filter_2026_11_09/` (`run.sh`, which calls `mirror.sh`,
  `prepare.py`, `git filter-repo`, `verify.py` and `scan.py`).

## Numbers

| | Before | After |
|---|---|---|
| Commits (integration + tags) | 1153 | 1113 (40 pruned, listed below) |
| Integration first-parent commits | 315 | 315 (order kept) |
| Ancestry | | every surviving commit's parents are the images of its old parents; a pruned parent stands for its own parents (a pruned merge that became degenerate keeps only the parent the other descends from) |
| Tags | 18 annotated | 18, each on the image of its commit; object type, tagger name/email/date kept; 10 tag messages rewritten by the message rules (old product name), exactly as the rules predict: v0.9.0–v0.9.2, v0.11.0, v0.11.1, v0.12.0, v1.0.0, v1.1.0–v1.1.2 |
| Refs | integration + 18 tags | the same; no `refs/pull`, `refs/notes`, `refs/replace`, no remote |
| Author and committer dates | | identical (raw, with time zone) for all 1113 surviving commits |
| Identities | all noreply | all noreply; mailmap built but empty (no real-name author on integration or tags) |
| Old-revision blobs stripped | | 122 (all 122 in `orig/`, 0 left in `repo/`) |
| Commit subjects changed by message rules | | 10 (old product name) |

## Pruned commits (emptied by the rules)

37 commits from the imported research histories, where everything they touched was research
content: the 30 research commits from 2026-10-16 to 2026-10-20 (both root commits among them),
the merge `4c29631` and the two subtree imports `da29892` and `ad823f4`.

Then three probe commits that held only the other app's A/B outputs: `cd42fed`, `077c5da` and
`b004192`. The competitor-comparison doc goes with its three commits: `956d8e1`, `65f12c9`
and the merge `12232b5`. `73eec83` (the cleanroom probe's neutral app id) is also pruned: the
text rule already made that change in the commits before it.

The full list with dates is printed by `verify.sh <run>`.

## Rules

- **Paths dropped from every commit** (`paths-drop.txt`):
  - `research/`, the old top-level `findings/`, `apks/`, `docs/handoffs/wispr-*-handoff.md` and
    the competitor-comparison doc;
  - the copied app lists and their generator;
  - the A/B probe and its cases, the `*_wispr.jsonl` files and the `wispr_parity` probe;
  - the vectors harness, the pre-split `postprocess.rs` and the eight removed postprocess modules;
  - the research probes under `tools/` and a committed `__pycache__`.
- **Old revisions dropped** (`drop-old-revisions.txt`; blob-id strip, so the file is absent from
  older commits):
  - `postprocess/apps.rs`, `postprocess/mod.rs` and `learn.rs` before D82;
  - `PolicyLists.kt` before 9d98254;
  - the bench fixtures and probe results before A4 `edb0393`;
  - `docs/design/app-design/ds/` before `7024ae9`;
  - the research repos' own `PROJECT_CONTEXT.md`, `CLAUDE.md`, `.gitignore` and `tools/`
    (commits that do not descend from the product root `cc76740`).

  A blob that is also in the integration HEAD tree is never stripped.
- **Text in blobs** (`replace-text.txt`, all regex):
  - the old product name → Murmur, in every spelling;
  - corpus cites (`M:`/`S:` line refs) → removed;
  - the other app's client function names and end-reason field → neutral words;
  - the old spoken-cleanup comment that quoted the other app's prompt → removed;
  - "port of …" and "mirrors …'s helper/lists" sentences → removed;
  - the other app's exe name in the cleanroom probe → the neutral id HEAD uses;
  - the A/B benchmark wording: "…-closeness" / "…-exact" → "target-…";
  - the copied UI strings → HEAD's own wording;
  - the demo sentence's name, coffee and traffic → invented ones.
- **Commit messages** (`replace-message.txt`): the old name, cites and function names.
- **Private** (`~/.cache/murmur-filter/private/replacements.txt`, uncommitted, mode 600): the
  owner's name parts, the personal email, the home username, and the real tailnet addresses
  and suffix. They are applied to blobs and messages, and the scan matches them by pattern.
- Every rule and scan phrase carries a `[x]` class, so once these files are part of the history
  they match neither themselves nor the scans.

## HEAD tree

The integration HEAD tree comes out byte-identical: 1789 files, 0 differ (same paths, modes and
blob ids; `verify.py` fails on any difference), and `headdiff.sh` prints nothing. The
HEAD-alignment commit on this branch already applied what the text rules would change:

- `CHANGELOG.md`: the old product name. The 0.8.0 entry now reads "Flow Clone → Murmur"
  (accepted). The entries for past releases now say Murmur on purpose, because the owner chose a
  full rename.
- Two copied UI strings in the design export (`PageScratchpad.dc.html`,
  `PageTransforms.dc.html`).
- `docs/public-release/history-drop.md` itself: that doc named the other app's client functions
  and end-reason field. It now uses the neutral words the rules write into old commits.

## Scans of the result (all blobs of all commits, all messages, all identities)

| Scan | Method | Result |
|---|---|---|
| Secrets | A7 `leak-patterns.txt` + `leak-allow.txt`, plus generic key/token/password assignments with entropy ≥ 4 | 0 open. Accepted: 7 hits on a Kotlin identifier (`Policy.kt`), 1 hit on the fixture's allowlisted fake key at its pre-A5 path, 2 hits on a one-letter test account name under `/home`, and 68 hits on synthetic 100.64.x.x test addresses. None of the 3 real tailnet addresses appears (private patterns) |
| Personal | private name, email, username, real tailnet addresses and suffix; the PC and tailnet hostnames; generic personal-mail domains; IL and international phone shapes; `*.ts.net`; non-noreply identities | 0 open. Accepted: 175 hits on third-party authors' addresses inside the licence texts (`THIRD-PARTY-LICENSES`, `third_party/licenses/`) |
| Wispr/legal | corpus cites, backend hosts, the prompt catalogue phrases, function and field names, port sentences, the copied UI strings, the old product name, RE tooling names, plus path rules (research tree, old findings, outputs, port vectors, copied lists, competitor doc) | 0 open |

Counted but not failed (info):

- the word "Wispr" in nominative use (comparisons, "A/B vs Wispr", the README credit);
- the device model (83 hits) and the TickTick project id (9 hits);
- 4 audio files: the public-domain JFK sample and 3 byte-sized stubs.

## Residuals

Decided: the CHANGELOG wording is accepted; the design-export strings and the bench prompt's
signature sentence (now in our own words in `tools/probes/bench/cleanup_system_prompt.txt`)
are fixed in HEAD. Left as is:

1. Section refs into the deleted research (for example "03 §7.2") stay in old commits. They
   point to text that is not in the public repo.
2. Old commits from before D82/A3/A4 no longer build, because their ported modules and copied
   lists are gone. HEAD builds as before.

## Owner sign-off items

1. This report: the pruned commits, the rules and the scan results above.
2. **Default branch name.** The push below publishes `integration` as `main`, the public
   repo's default branch. The alternative is to keep the name `integration`; in that case push
   `refs/heads/integration:refs/heads/integration` and set it as the default on GitHub.

## Step C (the first publish, run on 2026-11-09; later updates: [sync.md](sync.md))

Rerun on the then-current integration first (no `SOURCE_REF`), because the rules are
idempotent and the run directory is new each time:

```
bash tools/probes/prototypes/history_filter_2026_11_09/run.sh     # must end verify=0 scan=0
cd ~/.cache/murmur-filter/work/<new run>/repo
git remote add public git@github.com:bashycodes/murmur.git
git push public refs/heads/integration:refs/heads/main 'refs/tags/*:refs/tags/*'
```
