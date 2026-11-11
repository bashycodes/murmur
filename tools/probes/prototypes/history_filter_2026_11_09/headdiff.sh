#!/usr/bin/env bash
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
# headdiff.sh RUNDIR: word-level diff of every integration HEAD file the
# text rules changed (orig vs rewritten). For reviewing verify.py's list.
set -euo pipefail
run=$1
python3 - "$run" <<'EOF'
import subprocess, sys, difflib
run = sys.argv[1]
def g(repo, *a):
    return subprocess.run(["git", "-C", f"{run}/{repo}", *a], capture_output=True, text=True, errors="replace", check=True).stdout
def tree(repo):
    return {l.split("\t")[1]: l.split()[2] for l in g(repo, "ls-tree", "-r", "refs/heads/integration").splitlines()}
o, n = tree("orig"), tree("repo")
for p in sorted(x for x in o if o[x] != n.get(x)):
    a = g("orig", "cat-file", "blob", o[p]).splitlines()
    b = g("repo", "cat-file", "blob", n[p]).splitlines()
    print(f"== {p}")
    seen = 0
    for la, lb in ((x, y) for x, y in zip(a, b) if x != y):
        seen += 1
        if seen <= 4:
            print(f"   - {la[:200]}\n   + {lb[:200]}")
    print(f"   ({seen} lines changed, {len(a)} -> {len(b)} lines)")
EOF
