# History filter report (public B)

For owner sign-off before anything is pushed. Nothing has been pushed; the
rewritten repo exists only in a local scratch mirror with no remote.

- Source: `integration` at `8bb7e62` (A5 rename merged, = `origin/integration`), plus the 18 tags.
- Run: `~/.cache/murmur-filter/work/20261109T214510Z/` (`orig/` untouched, `repo/` rewritten).
- Tool: `tools/probes/prototypes/history_filter_2026_11_09/` (`run.sh`, which calls `mirror.sh`,
  `prepare.py`, `git filter-repo`, `verify.py` and `scan.py`).

## Numbers

| | Before | After |
|---|---|---|
| Commits (integration + tags) | 1148 | 1108 (40 pruned, listed below) |
| Integration first-parent commits | 310 | 310 (order kept) |
| Tags | 18 | 18 (each on the image of its commit) |
| Refs | integration + 18 tags | the same; no `refs/pull`, `refs/notes`, `refs/replace`, no remote |
| Author and committer dates | | identical (raw, with time zone) for all 1108 surviving commits |
| Identities | all noreply | all noreply; mailmap built but empty (no real-name author on integration or tags) |
| Old-revision blobs stripped | | 122 |
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

The integration HEAD tree comes out byte-identical (1771 files, 0 differ). A HEAD commit on
this branch already applied what the text rules would change: the old product name in
`CHANGELOG.md` (the 0.8.0 entry now reads "Flow Clone → Murmur", accepted), two copied UI
strings in the design export (`PageScratchpad.dc.html`, `PageTransforms.dc.html`) and the other
app's function and field names in `docs/public-release/history-drop.md`.

## Scans of the result (all blobs of all commits, all messages, all identities)

| Scan | Method | Result |
|---|---|---|
| Secrets | A7 `leak-patterns.txt` + `leak-allow.txt`, plus generic key/token/password assignments with entropy ≥ 4 | 0 open. Accepted: 7 hits on a Kotlin identifier (`Policy.kt`), 1 hit on the fixture's allowlisted fake key at its pre-A5 path, 2 hits on a one-letter test account name under `/home`, and 68 hits on synthetic 100.64.x.x test addresses. None of the 3 real tailnet addresses appears (private patterns) |
| Personal | private name, email, username, real tailnet addresses and suffix; the PC and tailnet hostnames; generic personal-mail domains; IL and international phone shapes; `*.ts.net`; non-noreply identities | 0 open. Accepted: 175 hits on third-party authors' addresses inside the licence texts (`THIRD-PARTY-LICENSES`, `third_party/licenses/`) |
| Wispr/legal | corpus cites, backend hosts, the prompt catalogue phrases, function and field names, port sentences, the copied UI strings, the old product name, RE tooling names, plus path rules (research tree, old findings, outputs, port vectors, copied lists, competitor doc) | 0 open |

Counted but not failed (info):

- the word "Wispr" in nominative use (comparisons, "A/B vs Wispr", the README credit);
- the device model (82 hits) and the TickTick project id (8 hits);
- 4 audio files: the public-domain JFK sample and 3 byte-sized stubs.

## Residuals

Decided: the CHANGELOG wording is accepted; the design-export strings and the bench prompt's
signature sentence (now in our own words in `tools/probes/bench/cleanup_system_prompt.txt`)
are fixed in HEAD. Left as is:

1. Section refs into the deleted research (for example "03 §7.2") stay in old commits. They
   point to text that is not in the public repo.
2. Old commits from before D82/A3/A4 no longer build, because their ported modules and copied
   lists are gone. HEAD builds as before.

## Step C (after sign-off; not run)

Rerun on the then-current integration first, because the rules are idempotent and the run
directory is new each time:

```
bash tools/probes/prototypes/history_filter_2026_11_09/run.sh     # must end verify=0 scan=0
cd ~/.cache/murmur-filter/work/<new run>/repo
git remote add public git@github.com:bashycodes/<public-repo>.git
git push public refs/heads/integration refs/tags/*
```
