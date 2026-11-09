#!/usr/bin/env python3
"""Compare the original mirror with the rewritten one (public B).

  verify.py --orig ORIG --new NEW

Checks, each printed as PASS/FAIL (exit 1 on any FAIL):
  * commit counts: new == orig - pruned (pruned = mapped to 0 in
    filter-repo's commit-map); the pruned commits are listed
  * every surviving commit keeps its author and committer date (raw,
    with time zone), author/committer name are noreply identities
  * integration's first-parent order is the original order minus pruned
  * the same tag names, each on the image of its original commit
  * no path of paths-drop.txt is left anywhere in the new history
  * integration's HEAD tree: identical except the listed paths (files the
    text rules touch, or old revisions that are HEAD content)
"""
import argparse
import fnmatch
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ZERO = "0" * 40
IDENTITY_OK = re.compile(r"(@users\.noreply\.github\.com|^noreply@github\.com|^noreply@anthropic\.com)$")
failed = []


def g(repo, *a):
    return subprocess.run(["git", "-C", repo, *a], capture_output=True, text=True, errors="surrogateescape",
                          check=True).stdout


def check(ok, what):
    print(("PASS " if ok else "FAIL ") + what)
    if not ok:
        failed.append(what)


def commits(repo):
    out = {}
    for line in g(repo, "log", "--all", "--date=raw", "--format=%H%x00%ad%x00%cd%x00%ae%x00%ce%x00%s").splitlines():
        h, ad, cd, ae, ce, s = line.split("\x00")
        out[h] = (ad, cd, ae, ce, s)
    return out


def tree(repo, ref):
    return {l.split("\t", 1)[1]: l.split("\t", 1)[0] for l in g(repo, "ls-tree", "-r", "--full-tree", ref).splitlines()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--orig", required=True)
    ap.add_argument("--new", required=True)
    a = ap.parse_args()
    old, new = commits(a.orig), commits(a.new)
    cmap = {}
    for line in pathlib.Path(a.new, "filter-repo", "commit-map").read_text().splitlines()[1:]:
        o, n = line.split()
        cmap[o] = n
    pruned = [o for o in old if cmap.get(o, ZERO) == ZERO]
    print(f"commits: orig {len(old)}, new {len(new)}, pruned {len(pruned)}")
    check(len(new) == len(old) - len(pruned), "commit count = orig - pruned")
    check(set(cmap[o] for o in old if o not in pruned) == set(new), "every new commit is the image of an original")
    print("pruned (emptied) commits, oldest first:")
    for o in sorted(pruned, key=lambda c: old[c][1].split()[0]):
        ad, cd, ae, ce, s = old[o]
        print(f"  {o[:10]} {ad.split()[0]} {s[:90]}")

    bad_dates = [o for o in old if o not in pruned and old[o][:2] != new[cmap[o]][:2]]
    check(not bad_dates, f"author+committer dates identical for {len(old) - len(pruned)} surviving commits"
          + (f" ({len(bad_dates)} differ, e.g. {bad_dates[0][:10]})" if bad_dates else ""))
    bad_ids = [n for n, v in new.items() if not (IDENTITY_OK.search(v[2]) and IDENTITY_OK.search(v[3]))]
    check(not bad_ids, "every author/committer email is a noreply identity")
    subj = sum(1 for o in old if o not in pruned and old[o][4] != new[cmap[o]][4])
    print(f"info: {subj} surviving commit subjects changed by the message rules")

    ofp = g(a.orig, "rev-list", "--first-parent", "--reverse", "refs/heads/integration").split()
    nfp = g(a.new, "rev-list", "--first-parent", "--reverse", "refs/heads/integration").split()
    check(nfp == [cmap[o] for o in ofp if o not in pruned], f"integration first-parent order kept ({len(nfp)} commits)")

    def tags(repo):
        return dict(l.split() for l in g(repo, "for-each-ref", "--format=%(refname) %(objectname:short=40)^{}",
                                         "refs/tags").replace("^{}", "").splitlines())

    otags = {r: g(a.orig, "rev-parse", r + "^{commit}").strip() for r in tags(a.orig)}
    ntags = {r: g(a.new, "rev-parse", r + "^{commit}").strip() for r in tags(a.new)}
    check(set(otags) == set(ntags), f"tag names identical ({len(otags)} tags)")
    check(all(ntags.get(r) == cmap.get(c) for r, c in otags.items()), "each tag points at the image of its commit")
    refs = g(a.new, "for-each-ref", "--format=%(refname)").split()
    check(all(r == "refs/heads/integration" or r.startswith("refs/tags/") for r in refs),
          f"only integration + tags ({len(refs)} refs; no refs/pull, refs/notes, refs/replace)")
    check(not g(a.new, "remote").strip(), "no remote configured")

    # Dropped paths are gone from every commit.
    rules = [l.strip() for l in (HERE / "paths-drop.txt").read_text().splitlines() if l.strip()]
    paths = set(g(a.new, "log", "--all", "--format=", "--name-only", "--no-renames", "-m").split("\n")) - {""}

    def dropped(p):
        for r in rules:
            if r.startswith("glob:") and fnmatch.fnmatchcase(p, r[5:]):
                return True
            if r.startswith("regex:") and re.search(r[6:], p):
                return True
            if r.endswith("/") and p.startswith(r) or p == r:
                return True
        return False

    left = sorted(p for p in paths if dropped(p))
    check(not left, f"no dropped path left in history ({len(left)} left{': ' + ', '.join(left[:5]) if left else ''})")

    ot, nt = tree(a.orig, "refs/heads/integration"), tree(a.new, "refs/heads/integration")
    changed = sorted(p for p in set(ot) | set(nt) if ot.get(p) != nt.get(p))
    print(f"integration HEAD tree: {len(ot)} files orig, {len(nt)} new, {len(changed)} differ")
    for p in changed:
        state = "removed" if p not in nt else "added" if p not in ot else "text rules"
        print(f"  HEAD {state}: {p}")
    check(all(p in ot and p in nt for p in changed), "HEAD tree has the same files (only text-rule edits)")
    print("verify:", "FAILED " + "; ".join(failed) if failed else "all checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
