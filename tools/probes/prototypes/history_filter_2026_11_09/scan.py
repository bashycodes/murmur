#!/usr/bin/env python3
"""Rescan a whole git history: every blob, commit message and identity.

Part of the public history filter (public B, see docs/public-release/history-filter-report.md).
Three scans, as in the pre-flip audits: secrets, personal, wispr/legal.

  scan.py --repo DIR [--private FILE] [--json OUT] [--context]

Patterns:
  scan-patterns.txt   `<category> <name> <python-regex>` lines (public)
  ../../../checks/leak-patterns.txt + leak-allow.txt  (A7 credential rules,
                      category "secrets", allowlist applied as leak-scan does)
  --private FILE      filter-repo replace-text file (uncommitted): every
                      left-hand side becomes a "personal" pattern. Its values
                      are never printed, only counts, commits and paths.
Path rules (category wispr-legal / personal): scan-paths.txt,
`<category> <name> <python-regex on the path>`.

Exit 0 when every category is clean, 1 otherwise. --context prints a short
snippet for non-personal hits (to classify them); personal hits never show
their value.
"""
import argparse
import collections
import fnmatch
import json
import math
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
CHECKS = HERE.parents[2] / "checks"
IDENTITY_OK = re.compile(r"(@users\.noreply\.github\.com|^noreply@github\.com|^noreply@anthropic\.com)$")
GENERIC_SECRET = re.compile(
    rb"(?i)(?:api[_-]?key|secret|token|passw(?:or)?d|bearer)[\"']?\s*[:=]\s*[\"']?([A-Za-z0-9_\-+/=.]{24,})")


def git(repo, *args, data=None):
    return subprocess.run(["git", "-C", repo, *args], input=data, capture_output=True, check=True).stdout


def entropy(s):
    c = collections.Counter(s)
    return -sum(n / len(s) * math.log2(n / len(s)) for n in c.values())


def load_lines(path, fields):
    out = []
    for raw in pathlib.Path(path).read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(" ", fields - 1)
        if len(parts) != fields:
            sys.exit(f"{path}: bad line {line[:40]!r}")
        out.append(parts)
    return out


def private_patterns(path):
    pats = []
    for i, raw in enumerate(pathlib.Path(path).read_text().splitlines()):
        if not raw.strip() or raw.startswith("#"):
            continue
        lhs = raw.split("==>", 1)[0]
        rx = lhs[len("regex:"):] if lhs.startswith("regex:") else re.escape(lhs)
        pats.append(("personal", f"private-{i}", re.compile(rx.encode())))
    return pats


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--private")
    ap.add_argument("--private-literals", help="uncommitted file, one literal per line (hostnames)")
    ap.add_argument("--accepted", help="`<name> <path-glob> <reason>` lines: hits counted as accepted")
    ap.add_argument("--json")
    ap.add_argument("--context", action="store_true")
    ap.add_argument("--max-show", type=int, default=12)
    a = ap.parse_args()

    pats = [(c, n, re.compile(r.encode())) for c, n, r in load_lines(HERE / "scan-patterns.txt", 3)]
    pats += [("secrets", n, re.compile(r.encode())) for n, r in load_lines(CHECKS / "leak-patterns.txt", 2)]
    allow = [(g, re.compile(r.encode())) for g, r in load_lines(CHECKS / "leak-allow.txt", 2)]
    leak_names = {n for n, _ in load_lines(CHECKS / "leak-patterns.txt", 2)}
    if a.private:
        pats += private_patterns(a.private)
    if a.private_literals:
        for i, lit in enumerate(l.strip() for l in pathlib.Path(a.private_literals).read_text().splitlines()):
            if lit:
                pats.append(("personal", f"private-literal-{i}", re.compile(rb"(?i)\b" + re.escape(lit.encode()) + rb"\b")))
    accepted = []
    if a.accepted:
        for raw in pathlib.Path(a.accepted).read_text().splitlines():
            if raw.strip() and not raw.startswith("#"):
                n, glob, reason = raw.split(" ", 2)
                accepted.append((n, glob, reason))
    path_rules = [(c, n, re.compile(r)) for c, n, r in load_lines(HERE / "scan-paths.txt", 3)]

    # blob -> first (commit, path) that introduced it, from --raw logs.
    first = {}
    paths = collections.defaultdict(set)
    commit = None
    raw = git(a.repo, "log", "--all", "--raw", "--no-abbrev", "--no-renames", "-m", "--reverse",
              "--format=C %H").decode("utf-8", "surrogateescape")
    for line in raw.splitlines():
        if line.startswith("C "):
            commit = line[2:12]
        elif line.startswith(":"):
            meta, path = line.split("\t", 1)
            blob = meta.split()[3]
            if set(blob) == {"0"}:
                continue
            paths[blob].add(path)
            first.setdefault(blob, (commit, path))

    hits = collections.defaultdict(lambda: {"category": "", "occurrences": 0, "where": []})

    def record(cat, name, where, path, ctx):
        for n, glob, reason in accepted:
            if n == name and fnmatch.fnmatchcase(path, glob):
                cat, name = "accepted", f"{name} [{reason}]"
                break
        h = hits[name]
        h["category"] = cat
        h["occurrences"] += 1
        if len(h["where"]) < 400:
            item = where
            if a.context and cat != "personal":
                item += "  | " + ctx
            if item not in h["where"]:
                h["where"].append(item)

    def allowed(path, value):
        return any(fnmatch.fnmatchcase(path, g) and r.search(value) for g, r in allow)

    def scan_bytes(data, where, path):
        for cat, name, rx in pats:
            for m in rx.finditer(data):
                if name in leak_names and allowed(path, m.group(0)):
                    continue
                s = max(0, m.start() - 50)
                ctx = data[s:m.end() + 50].decode("utf-8", "replace").replace("\n", "⏎")
                record(cat, name, where, path, ctx)
        for m in GENERIC_SECRET.finditer(data):
            v = m.group(1).decode("latin-1")
            if entropy(v) >= 4.0 and not allowed(path, m.group(1)):
                record("secrets", "generic-high-entropy", where, path, v[:4] + "…")

    # Blobs.
    blobs = sorted(paths)
    proc = subprocess.Popen(["git", "-C", a.repo, "cat-file", "--batch"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE)
    for blob in blobs:
        proc.stdin.write(blob.encode() + b"\n")
        proc.stdin.flush()
        header = proc.stdout.readline().split()
        size = int(header[2])
        data = proc.stdout.read(size)
        proc.stdout.read(1)
        c, p = first[blob]
        scan_bytes(data, f"{c} {p}", p)
    proc.stdin.close()
    proc.wait()

    # Paths.
    all_paths = set().union(*paths.values()) if paths else set()
    for p in sorted(all_paths):
        for cat, name, rx in path_rules:
            if rx.search(p):
                record(cat, name, p, p, p)

    # Messages and identities.
    log = git(a.repo, "log", "--all", "--format=%H%x00%an%x00%ae%x00%cn%x00%ce%x00%B%x01")
    ncommits = 0
    for entry in log.split(b"\x01"):
        entry = entry.strip(b"\n")
        if not entry:
            continue
        ncommits += 1
        h, an, ae, cn, ce, body = entry.split(b"\x00", 5)
        h = h.decode()[:10]
        scan_bytes(body, f"{h} <message>", "<message>")
        scan_bytes(an + b" " + ae + b" " + cn + b" " + ce, f"{h} <identity>", "<identity>")
        for e in (ae, ce):
            if not IDENTITY_OK.search(e.decode()):
                record("personal", "non-noreply-identity", f"{h} <identity>", "<identity>", "")

    cats = collections.defaultdict(int)
    for name, h in hits.items():
        cats[h["category"]] += h["occurrences"]
    summary = {"commits": ncommits, "blobs": len(blobs), "paths": len(all_paths),
               "categories": {c: cats.get(c, 0) for c in ("secrets", "personal", "wispr-legal", "accepted", "info")},
               "hits": {n: dict(h) for n, h in sorted(hits.items())}}
    if a.json:
        pathlib.Path(a.json).write_text(json.dumps(summary, indent=1) + "\n")
    print(f"scanned {ncommits} commits, {len(blobs)} blobs, {len(all_paths)} paths")
    for c, n in summary["categories"].items():
        print(f"  {c}: {n} hits")
    for name, h in sorted(hits.items(), key=lambda kv: (kv[1]["category"], kv[0])):
        print(f"[{h['category']}] {name}: {h['occurrences']} occurrences in {len(h['where'])} places")
        for w in h["where"][:a.max_show]:
            print("    " + w[:300])
    return 1 if any(h["category"] not in ("info", "accepted") for h in hits.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
