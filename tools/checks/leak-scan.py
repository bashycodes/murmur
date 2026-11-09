#!/usr/bin/env python3
"""Repo-local leak scan: credentials, tailnet IPs and home paths.

Patterns live in leak-patterns.txt, fake test values in leak-allow.txt (both
next to this file). Complements the owner's global `leak-scan` pre-push hook,
which only knows private project names; this one runs in CI too, so a merge
made on github.com (no local hook) is still scanned.

  leak-scan.py --tree            every tracked file at the working tree
  leak-scan.py BASE..HEAD        lines added in the range, plus its commit
                                 messages and author/committer emails
  leak-scan.py --stdin           lines on stdin (tests)

Exit 0 clean, 1 on a leak, 2 on usage/config error. Hits print the file,
line and pattern name with the matched value masked, so a CI log never
carries the secret it caught.
"""
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
# Commit identities must be GitHub noreply addresses ("Keep my email private");
# noreply@github.com is the committer of merges made on github.com.
IDENTITY_OK = re.compile(r"(@users\.noreply\.github\.com|^noreply@github\.com|^noreply@anthropic\.com)$")


def load_patterns(path):
    out = []
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        name, _, rx = line.partition(" ")
        out.append((name, re.compile(rx)))
    return out


def load_allow(path):
    return [re.compile(l.strip()) for l in path.read_text().splitlines()
            if l.strip() and not l.strip().startswith("#")]


def mask(value):
    return value[:6] + "…" if len(value) > 6 else value


class Scanner:
    def __init__(self, patterns, allow):
        self.patterns, self.allow, self.hits = patterns, allow, 0

    def line(self, where, text):
        for name, rx in self.patterns:
            for m in rx.finditer(text):
                if any(a.search(m.group(0)) for a in self.allow):
                    continue
                self.hits += 1
                print(f"LEAK {where}: {name} {mask(m.group(0))}", file=sys.stderr)


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True,
                          text=True, errors="replace").stdout


def scan_tree(sc):
    files = git("ls-files", "-z").split("\0")
    for f in filter(None, files):
        p = pathlib.Path(f)
        if not p.is_file():
            continue
        data = p.read_bytes()
        if b"\0" in data[:8192]:
            continue  # binary
        for n, text in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
            sc.line(f"{f}:{n}", text)


def scan_range(sc, rng):
    # Two-dot tree diff: covers merge commits, whose resolution `git log -p`
    # would hide.
    base, _, head = rng.partition("..")
    f, n = "?", 0
    for raw in git("diff", "--no-color", "--no-ext-diff", "-U0", base, head).splitlines():
        if raw.startswith("+++ "):
            f = raw[6:] if raw.startswith("+++ b/") else raw[4:]
        elif raw.startswith("@@"):
            m = re.search(r"\+(\d+)", raw)
            n = int(m.group(1)) if m else 0
        elif raw.startswith("+"):
            sc.line(f"{f}:{n}", raw[1:])
            n += 1
    log = git("log", "--format=%x00%h%x01%ae%x01%ce%x01%B", rng)
    for rec in filter(None, log.split("\0")):
        sha, ae, ce, body = rec.split("\x01", 3)
        for kind, email in (("author", ae), ("committer", ce)):
            if not IDENTITY_OK.search(email):
                sc.hits += 1
                print(f"LEAK commit {sha}: {kind} email is not a noreply address",
                      file=sys.stderr)
        for i, text in enumerate(body.splitlines(), 1):
            sc.line(f"commit {sha} message:{i}", text)


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    sc = Scanner(load_patterns(HERE / "leak-patterns.txt"), load_allow(HERE / "leak-allow.txt"))
    arg = argv[1]
    if arg == "--stdin":
        for n, text in enumerate(sys.stdin.read().splitlines(), 1):
            sc.line(f"stdin:{n}", text)
    elif arg == "--tree":
        scan_tree(sc)
    elif ".." in arg:
        scan_range(sc, arg)
    else:
        print(__doc__, file=sys.stderr)
        return 2
    if sc.hits:
        print(f"leak-scan: {sc.hits} hit(s). Fake test values must carry a marker "
              f"from tools/checks/leak-allow.txt (e.g. LEAKCHECK, FAKE).", file=sys.stderr)
        return 1
    print("leak-scan: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
