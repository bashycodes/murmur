#!/usr/bin/env python3
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
"""Exploration helpers used while writing the filter rules (kept for audit).

  explore.py REPO roots            root commits and the paths of the
                                   commits that do not descend from the
                                   product root (imported RE histories)
  explore.py REPO paths REGEX      every path ever in history matching REGEX,
                                   with the number of revisions
  explore.py REPO grep REGEX [PATHREGEX]
                                   blobs matching REGEX (path, first commit,
                                   a snippet); never use for personal values
"""
import collections
import re
import subprocess as sp
import sys


def g(repo, *a):
    return sp.run(["git", "-C", repo, *a], capture_output=True, text=True, errors="replace", check=True).stdout


def raw_changes(repo):
    commit = None
    for line in g(repo, "log", "--all", "--raw", "--no-abbrev", "--no-renames", "-m", "--reverse",
                  "--format=C %H").splitlines():
        if line.startswith("C "):
            commit = line[2:]
        elif line.startswith(":"):
            meta, path = line.split("\t", 1)
            yield commit, meta.split()[3], path


def main():
    repo, cmd, *rest = sys.argv[1:]
    if cmd == "roots":
        roots = g(repo, "rev-list", "--max-parents=0", "--all").split()
        allc = g(repo, "rev-list", "--all").split()
        dated = sorted(roots, key=lambda r: g(repo, "log", "-1", "--format=%at", r))
        for r in dated:
            print(g(repo, "log", "-1", "--format=%h %ad %s", "--date=short", r).strip())
        main_root = dated[0] if len(rest) == 0 else rest[0]
        desc = set(g(repo, "rev-list", "--ancestry-path", f"{main_root}..HEAD", "--all").split()) | {main_root}
        # descendants of main root among all commits
        desc = set()
        children = collections.defaultdict(list)
        for line in g(repo, "rev-list", "--all", "--parents").splitlines():
            c, *ps = line.split()
            for p in ps:
                children[p].append(c)
        stack = [main_root]
        while stack:
            c = stack.pop()
            if c in desc:
                continue
            desc.add(c)
            stack.extend(children[c])
        other = [c for c in allc if c not in desc]
        print(f"all {len(allc)}, product-root descendants {len(desc)}, other {len(other)}")
        tops = collections.Counter()
        for c in other:
            for p in g(repo, "show", "--format=", "--name-only", "--no-renames", c).splitlines():
                if p:
                    tops["/".join(p.split("/")[:2])] += 1
        for k, v in sorted(tops.items()):
            print(f"{v:5} {k}")
        if len(rest) > 1:
            for c in other:
                names = g(repo, "show", "--format=", "--name-only", "--no-renames", c).splitlines()
                if any(n.startswith(rest[1]) for n in names):
                    print(g(repo, "log", "-1", "--format=%h %ad %P %s", "--date=short", c).strip())
    elif cmd == "paths":
        rx = re.compile(rest[0])
        n = collections.Counter()
        for _, blob, path in raw_changes(repo):
            if rx.search(path) and set(blob) != {"0"}:
                n[path] += 1
        for p, v in sorted(n.items()):
            print(f"{v:4} {p}")
    elif cmd == "grep":
        rx = re.compile(rest[0].encode())
        prx = re.compile(rest[1]) if len(rest) > 1 else None
        seen = {}
        for c, blob, path in raw_changes(repo):
            if set(blob) == {"0"} or blob in seen or (prx and not prx.search(path)):
                continue
            seen[blob] = (c, path)
        out = collections.defaultdict(list)
        for blob, (c, path) in seen.items():
            data = sp.run(["git", "-C", repo, "cat-file", "blob", blob], capture_output=True).stdout
            for m in rx.finditer(data):
                s = max(0, m.start() - 60)
                out[path].append((c[:10], data[s:m.end() + 60].decode("utf-8", "replace").replace("\n", "⏎")))
        for p in sorted(out):
            print(f"== {p} ({len(out[p])})")
            for c, snip in out[p][:4]:
                print(f"   {c} {snip}")


if __name__ == "__main__":
    main()
