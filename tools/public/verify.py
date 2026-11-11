#!/usr/bin/env python3
"""Compare the original mirror with the rewritten one (tools/public filter).

  verify.py --orig ORIG --new NEW

Checks, each printed as PASS/FAIL (exit 1 on any FAIL):
  * commit counts: new == orig - pruned (pruned = mapped to 0 in
    filter-repo's commit-map); the pruned commits are listed
  * every surviving commit keeps its author and committer date (raw,
    with time zone), author/committer name are noreply identities
  * integration's first-parent order is the original order minus pruned
  * full ancestry: every surviving commit's parents are the images of its
    old parents, a pruned parent collapsed to the images of its own parents
  * the same tag names, each on the image of its original commit; annotated
    tags keep type, tagger name/email/date, and their message (or, with
    --replace-message, exactly the rewritten message; rewritten ones listed)
  * no path of paths-drop.txt is left anywhere in the new history
  * with --strip-blobs, none of the listed old-revision blobs is in NEW
  * integration's HEAD tree is byte-identical (same paths, modes, blob ids)
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


def dedup(xs):
    out = []
    for x in xs:
        if x not in out:
            out.append(x)
    return out


def load_message_rules(path):
    """filter-repo --replace-message rules, applied in its order: literals, then regexes."""
    lits, rxs = [], []
    for line in pathlib.Path(path).read_bytes().splitlines():
        if not line.strip() or line.startswith(b"#"):
            continue
        old, sep, new = line.rpartition(b"==>")
        if not sep:
            old, new = line, b"***REMOVED***"
        if old.startswith(b"regex:"):
            rxs.append((re.compile(old[6:]), new))
        elif old.startswith(b"glob:"):
            rxs.append((re.compile(fnmatch.translate(old[5:].decode()).encode()), new))
        else:
            lits.append((old[8:] if old.startswith(b"literal:") else old, new))
    return lits, rxs


def apply_rules(rules, data):
    lits, rxs = rules
    for old, new in lits:
        data = data.replace(old, new)
    for rx, new in rxs:
        data = rx.sub(new, data)
    return data


def tagobj(repo, ref):
    """(object type, {type, tag, tagger header fields}, message) of a tag ref."""
    typ = g(repo, "cat-file", "-t", ref).strip()
    if typ != "tag":
        return typ, {}, b""
    raw = subprocess.run(["git", "-C", repo, "cat-file", "tag", ref], capture_output=True, check=True).stdout
    head, _, msg = raw.partition(b"\n\n")
    fields = dict(l.split(b" ", 1) for l in head.splitlines())
    return typ, {k: fields.get(k) for k in (b"type", b"tag", b"tagger")}, msg


def batch_check(repo, ids):
    out = subprocess.run(["git", "-C", repo, "cat-file", "--batch-check"], input="".join(i + "\n" for i in ids),
                         capture_output=True, text=True, check=True).stdout.splitlines()
    return [l.split()[0] for l in out if not l.endswith(" missing")]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--orig", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--strip-blobs", help="the run's inputs/strip-blobs.txt")
    ap.add_argument("--replace-message", help="the run's inputs/replace-message.txt (private; never printed)")
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

    def parents(repo):
        return {l.split()[0]: l.split()[1:] for l in g(repo, "rev-list", "--all", "--parents").splitlines()}

    opar, npar = parents(a.orig), parents(a.new)
    prunedset, memo = set(pruned), {}

    def is_anc(x, y):
        return subprocess.run(["git", "-C", a.new, "merge-base", "--is-ancestor", x, y]).returncode == 0

    def expected(o):
        """NEW parents of a surviving old commit: its parents' images, deduplicated."""
        return dedup(x for p in opar[o] for x in images(p))

    def images(o):
        """What an old commit stands for in NEW: its image or, if pruned, its parents' images,
        minus any that is an ancestor of another (a pruned merge became degenerate)."""
        if o not in memo:
            if o not in prunedset:
                memo[o] = [cmap[o]]
            else:
                got = expected(o)
                memo[o] = [x for x in got if not any(y != x and is_anc(x, y) for y in got)]
        return memo[o]

    sys.setrecursionlimit(10000)
    bad_par = [o for o in old if o not in prunedset and npar[cmap[o]] != expected(o)]
    for o in bad_par:
        want = expected(o)
        print(f"  ancestry {o[:10]} -> {cmap[o][:10]}: parents {[p[:10] for p in npar[cmap[o]]]},"
              f" expected {[p[:10] for p in want]} (old {[p[:10] + ('*' if p in prunedset else '') for p in opar[o]]})")
    check(not bad_par, f"full ancestry kept: parents of all {len(old) - len(pruned)} surviving commits are the"
          " images of their old parents" + (f" ({len(bad_par)} differ, e.g. {bad_par[0][:10]})" if bad_par else ""))

    def tags(repo):
        return dict(l.split() for l in g(repo, "for-each-ref", "--format=%(refname) %(objectname:short=40)^{}",
                                         "refs/tags").replace("^{}", "").splitlines())

    otags = {r: g(a.orig, "rev-parse", r + "^{commit}").strip() for r in tags(a.orig)}
    ntags = {r: g(a.new, "rev-parse", r + "^{commit}").strip() for r in tags(a.new)}
    check(set(otags) == set(ntags), f"tag names identical ({len(otags)} tags)")
    check(all(ntags.get(r) == cmap.get(c) for r, c in otags.items()), "each tag points at the image of its commit")
    rules = load_message_rules(a.replace_message) if a.replace_message else None
    bad_tag, rewritten, nannot = [], [], 0
    for r in sorted(set(otags) & set(ntags)):
        (ot_, of, om), (nt_, nf, nm) = tagobj(a.orig, r), tagobj(a.new, r)
        nannot += ot_ == "tag"
        want = apply_rules(rules, om) if rules is not None else om
        if ot_ != nt_ or of != nf or nm != want:
            bad_tag.append(r)
        elif nm != om:
            rewritten.append(r)
    check(not bad_tag, f"tags keep object type, tag type and name, tagger name/email/date and message"
          f" ({nannot} annotated)" + (f"; differ: {', '.join(bad_tag)}" if bad_tag else ""))
    print("info: tag messages rewritten by the message rules (as the rules predict): "
          + (", ".join(rewritten) if rewritten else "none"))
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

    if a.strip_blobs:
        ids = [l.split()[0] for l in pathlib.Path(a.strip_blobs).read_text().splitlines() if l.strip()]
        in_orig, left = batch_check(a.orig, ids), batch_check(a.new, ids)
        check(not left and len(in_orig) == len(ids),
              f"old-revision blobs: {len(ids)} listed, {len(in_orig)} in orig, {len(left)} left in new")

    ot, nt = tree(a.orig, "refs/heads/integration"), tree(a.new, "refs/heads/integration")
    changed = sorted(p for p in set(ot) | set(nt) if ot.get(p) != nt.get(p))
    print(f"integration HEAD tree: {len(ot)} files orig, {len(nt)} new, {len(changed)} differ")
    for p in changed:
        state = "removed" if p not in nt else "added" if p not in ot else "text rules"
        print(f"  HEAD {state}: {p}")
    check(not changed, "HEAD tree byte-identical (same paths, modes and blob ids)")
    print("verify:", "FAILED " + "; ".join(failed) if failed else "all checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
