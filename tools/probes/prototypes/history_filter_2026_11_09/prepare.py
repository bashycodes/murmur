#!/usr/bin/env python3
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
"""Build the per-run filter-repo inputs from the committed rules.

  prepare.py --repo MIRROR --out RUNDIR [--private FILE]

Writes into RUNDIR (mode 600, the private values are inside):
  strip-blobs.txt     blob ids for --strip-blobs-with-ids, from
                      drop-old-revisions.txt: every blob a listed path had
                      in a commit that does not contain the path's
                      "good-from" commit, unless that same blob is also the
                      path's content in a commit that does
  strip-blobs.tsv     the same with the path and the first commit (audit)
  replace-text.txt    replace-text.txt + the private file (blobs)
  replace-message.txt replace-message.txt + the private file (messages)
  mailmap             every identity whose email is not a GitHub/Anthropic
                      noreply, or whose name is a private value, mapped to
                      the repo's own noreply identity
"""
import argparse
import collections
import os
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
IDENTITY_OK = re.compile(r"(@users\.noreply\.github\.com|^noreply@github\.com|^noreply@anthropic\.com)$")
OWNER = "bashycodes <73243853+bashycodes@users.noreply.github.com>"


def g(repo, *a):
    return subprocess.run(["git", "-C", repo, *a], capture_output=True, text=True, errors="surrogateescape",
                          check=True).stdout


def rules(path):
    out = []
    for raw in path.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            out.append(line.split())
    return out


def write(path, text):
    # Created 0600 from the start: the private values never sit in a wider-mode file.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)  # an existing file keeps its old mode under O_CREAT
    with os.fdopen(fd, "w") as f:
        f.write(text)


def descendants(repo, root):
    children = collections.defaultdict(list)
    for line in g(repo, "rev-list", "--all", "--parents").splitlines():
        c, *ps = line.split()
        for p in ps:
            children[p].append(c)
    seen, stack = set(), [root]
    while stack:
        c = stack.pop()
        if c not in seen:
            seen.add(c)
            stack.extend(children[c])
    return seen


def strip_blobs(repo, out):
    want = {}
    for path, good in rules(HERE / "drop-old-revisions.txt"):
        want[path] = g(repo, "rev-parse", "--verify", good + "^{commit}").strip()
    good_sets = {sha: descendants(repo, sha) for sha in set(want.values())}
    bad, keep = {}, collections.defaultdict(set)
    commit = None
    for line in g(repo, "log", "--all", "--raw", "--no-abbrev", "--no-renames", "-m", "--format=C %H").splitlines():
        if line.startswith("C "):
            commit = line[2:]
            continue
        if not line.startswith(":"):
            continue
        meta, path = line.split("\t", 1)
        blob = meta.split()[3]
        rule = next((r for r in want if path == r or (r.endswith("/") and path.startswith(r))), None)
        if rule is None or set(blob) == {"0"}:
            continue
        if commit in good_sets[want[rule]]:
            keep[path].add(blob)
        else:
            bad.setdefault((path, blob), (commit, rule))
    rows = sorted((p, b, c, r) for (p, b), (c, r) in bad.items() if b not in keep[p])
    head = {l.split()[2] for l in g(repo, "ls-tree", "-r", "refs/heads/integration").splitlines()}
    clash = {b for _, b, _, _ in rows} & head
    if clash:
        print(f"  {len(clash)} old-revision blobs are also in the integration HEAD tree: kept")
    rows = [r for r in rows if r[1] not in clash]
    blobs = sorted({b for _, b, _, _ in rows})
    write(out / "strip-blobs.txt", "".join(f"{b}\n" for b in blobs))
    write(out / "strip-blobs.tsv", "".join(f"{p}\t{b}\t{c[:10]}\n" for p, b, c, _ in rows))
    per = collections.Counter(r for _, _, _, r in rows)
    for r in want:
        print(f"  strip {per[r]:4} old revisions under {r}")
    return len(blobs)


def private_lines(private):
    if not private:
        return []
    return [l for l in pathlib.Path(private).read_text().splitlines() if l.strip() and not l.startswith("#")]


def mailmap(repo, out, private):
    priv = private_lines(private)
    priv_rx = [re.compile(l.split("==>")[0][6:] if l.startswith("regex:") else re.escape(l.split("==>")[0]))
               for l in priv]
    ids = set()
    for line in g(repo, "log", "--all", "--format=%an%x00%ae%n%cn%x00%ce").splitlines():
        ids.add(tuple(line.split("\x00")))
    lines = []
    for name, email in sorted(ids):
        bad_email = not IDENTITY_OK.search(email)
        bad_name = any(r.search(name) for r in priv_rx)
        if bad_email or bad_name:
            lines.append(f"{OWNER} {name} <{email}>")
    write(out / "mailmap", "".join(l + "\n" for l in lines))
    return len(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ap.add_argument("--private")
    a = ap.parse_args()
    priv = private_lines(a.private)
    n = strip_blobs(a.repo, a.out)
    for name in ("replace-text.txt", "replace-message.txt"):
        public = [l for l in (HERE / name).read_text().splitlines() if l.strip() and not l.startswith("#")]
        write(a.out / name, "\n".join(public + priv) + "\n")
    m = mailmap(a.repo, a.out, a.private)
    print(f"prepare: {n} blobs to strip, {len(priv)} private replacements, {m} identities mapped")


if __name__ == "__main__":
    sys.exit(main())
