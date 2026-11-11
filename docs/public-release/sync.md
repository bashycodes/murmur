# Public sync: private integration → public main

The public repo `bashycodes/murmur` has one branch, `main`, which is the
filtered history of the private `integration` branch, plus the release tags.
The work still happens on `integration` in the private repo. `tools/public/sync.sh`
brings the public `main` up to date. It filters the history again and pushes
only the commits that are new. It never rewrites what is already public.

The tool was promoted from the 2026-11-09 prototype
(`tools/probes/prototypes/history_filter_2026_11_09/`, frozen). The rules and
the first publish are described in [history-filter-report.md](history-filter-report.md).

## Run it

Needs `git filter-repo`, python3, about 2 GB of free memory, and the private
replacement files in `~/.cache/murmur-filter/private/`. They are not committed,
and their contents are never printed.

```
tools/public/sync.sh                               # dry run (the default)
tools/public/sync.sh --run <run dir> --push        # push what that dry run showed
tools/public/sync.sh --run <run dir> --push --tags v1.2.0   # with a new release tag
```

A dry run does these steps:

1. Fetches `origin/integration`.
2. Runs `tools/public/filter.sh` into a new run directory,
   `~/.cache/murmur-filter/work/<UTC time>/`. Runs are never reused or
   overwritten. The run must pass `verify.py` and the full-history `scan.py`
   (secrets, personal, wispr/legal).
3. Checks the run against the public repo, read-only (`git ls-remote`), and
   rescans the new range with `tools/checks/leak-scan.py`.
4. Prints the range `<public main>..<filtered tip>`, its commit count, the
   first-parent subjects, the tags not yet published, and the exact command
   that pushes this run.

With `--push`, the tool first runs `git push public <tip>:refs/heads/main`.
Then it pushes each tag you name with `--tags` on its own, as
`git push public refs/tags/vX.Y.Z:refs/tags/vX.Y.Z`. It never uses `--force`,
`--all` or `--tags`. GitHub's own non-fast-forward check still applies.

The tags go one per push because of a GitHub limit: a push that carries more
than three tags creates no push event. No workflow runs for it, so no release
job runs either.

`--run DIR` reuses a run that has passed, so the push matches what the dry run
showed. The checks against the public repo run again at push time. The run
must still match `origin/integration`, which is fetched again first. If
integration has moved since the run, the sync refuses. Either filter again
(drop `--run`), or pass `--allow-stale` to publish the older state on purpose.
The fast-forward then stays a fast-forward, and the next sync brings in the
rest.

Release flow: run `tools/release.sh X.Y.Z --apply`, push to the private repo,
then run `sync.sh --tags vX.Y.Z` and, after that, `--push`. The public CI then
builds the tag and attaches the APKs and the installers to the public release.

### Backfilling the releases of the first publish

The first publish on 2026-11-09 pushed `main` and all 18 tags in one push,
so the tags got no push event and the public repo has no releases for them.
Both build workflows have `workflow_dispatch`. A dispatch on a tag ref runs the
workflow file at that tag with `github.ref_type == 'tag'`, so the release job
runs too. To backfill one tag:

```
gh workflow run android-build.yml --repo bashycodes/murmur --ref v1.1.2
gh workflow run windows-build.yml --repo bashycodes/murmur --ref v1.1.2
```

Every tag from v0.2.0 to v1.1.2 has both `workflow_dispatch` and the release
job. Older tags may still fail to build, because the filter removed modules
that their trees needed (history-filter-report.md, "Residuals"). Those tags
stay without assets. This doc only describes the backfill; it has not been run.

## What it refuses (exit 1, nothing pushed)

| Refusal | Meaning |
|---|---|
| filter run failed verify or scan / no `ok` marker | The run's `verify.txt` or `scan.txt` names the problem. A common case is a new integration commit with text that the rules would change in the HEAD tree: fix it on integration. |
| run filtered a commit not on `origin/integration` | The run is a preview (`SOURCE_REF=feat/x filter.sh`). Only merged work is published. |
| stale run | `--run` names a run of an older integration. Filter again, or pass `--allow-stale`. |
| published main is not in the filtered history | The filter no longer reproduces the published SHAs. See "Determinism". |
| published main is not an ancestor of the filtered tip | Not a fast-forward. Someone pushed to the public `main` directly. |
| published tag `vX` is another object here | A published tag moved, or its message rules changed. |
| leak-scan found hits | A credential, a tailnet address, a home path or a non-noreply identity is in the new range. |
| tag is not in the run / not on the new main | `--tags` named a tag that has not been made yet, or a tag that is not on integration. |

## Determinism

The sync works only because the filter is deterministic. The same input
commits, rules and private replacements give the same output SHAs. This was
checked on 2026-11-11: a rerun on `integration` `085e407` (8 commits after the
published source `8e5c954`) mapped `8e5c954` to `fad649b`, which is exactly
the published `main`. All 18 published tag objects came out identical, and the
new tip `21b25f2` is a pure fast-forward of 8 commits.

filter-repo keeps author and committer dates, and an empty mailmap leaves the
identities alone. The rules see only what is in each commit. So a rerun changes
old commits only when one of these changes:

- the committed rules in `tools/public/`, such as `replace-text.txt`,
  `replace-message.txt`, `paths-drop.txt` or `drop-old-revisions.txt`;
- the private replacements;
- the `git filter-repo` version;
- the old-revision strip set. `prepare.py` computes this set over the whole
  history. A new commit that brings back an old stripped blob at a listed path,
  such as a revert, keeps that blob in the old commits too.

Each of these changes shows up as the "not in the filtered history" refusal.
Nothing is pushed.

## Recovery

- **Not reproduced.** Use `git log` on `tools/public/` to find the change, then
  check the private file's mtime and `git filter-repo --version`. If the change
  was not meant to affect old commits, revert it or narrow it. If it was meant
  to (a new leak found in old history), the public history has to be
  rewritten. That means a force-push of `main` and the tags, which the tool
  deliberately cannot do. It is an owner decision.
- **Not a fast-forward.** Bring the public-only commits into private
  `integration` by hand: cherry-pick them, then check that the result filters
  back to the same SHAs. Otherwise the owner decides to drop them.
- **Leak hit.** A fix commit on integration is not enough, because the commit
  that added the leak is still in the range. Add a filter rule (a
  `replace-text.txt` line, or a private replacement for a private value) that
  removes the value. If the value appears only in unpublished commits, the
  published SHAs do not change, and the next dry run shows whether that holds.
  Then fix HEAD too, because verify requires the HEAD tree to come out
  unchanged. Rotate any real credential.
- **A push failed halfway.** Each push lands whole or not at all. If `main`
  landed but a tag did not, rerun the same command: `main` is then up to date,
  and only the missing tags are pushed.

## Open decision: archiving the private repo

The private repo stays the working repo. Tickets, branches, reviews and CI all
run there, and the public repo only receives filtered fast-forwards. Archiving
the private repo and working in public directly has not been decided. That
switch would retire this sync. (The private repo's name is not written here,
because the filter rules rewrite it.)
