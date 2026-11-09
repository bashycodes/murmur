#!/usr/bin/env python3
"""Repo-local leak scan: credentials, tailnet IPs and home paths.

Patterns live in leak-patterns.txt, allowed fakes in leak-allow.txt (both
next to this file). Complements the owner's global `leak-scan` pre-push hook,
which only knows private project names; this one runs in CI too, so a merge
made on github.com (no local hook) is still scanned.

  leak-scan.py --tree            every tracked file of the repo (any cwd)
  leak-scan.py BASE..HEAD        lines added in the range, plus its commit
  leak-scan.py BASE...HEAD       messages and author/committer emails
                                 (three dots: from merge-base(BASE, HEAD))
  leak-scan.py --stdin           lines on stdin (tests)

Exit 0 clean, 1 on a leak, 2 on usage, config or git error. A hit prints the
pattern name, the location and at most 4 characters of the value (tailnet
addresses as 100.x.x.x), so a CI log never carries the secret it caught.
"""
import fnmatch
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
# Commit identities must be GitHub noreply addresses ("Keep my email private");
# noreply@github.com is the committer of merges made on github.com, and
# noreply@anthropic.com is the identity of commits made by coding agents
# (their Co-Authored-By trailer uses the same address).
IDENTITY_OK = re.compile(r"(@users\.noreply\.github\.com|^noreply@github\.com|^noreply@anthropic\.com)$")


class UsageError(Exception):
    pass


def load_rules(path):
    """`<first-field> <regex>` lines of a rules file, regex compiled."""
    rules = []
    try:
        text = path.read_text()
    except OSError as err:
        raise UsageError(f"cannot read {path}: {err.strerror}") from err
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        field, _, regex = line.partition(" ")
        if not regex:
            raise UsageError(f"{path.name}:{lineno}: expected `<field> <regex>`")
        try:
            rules.append((field, re.compile(regex)))
        except re.error as err:
            raise UsageError(f"{path.name}:{lineno}: bad regex: {err}") from err
    if not rules:
        raise UsageError(f"{path.name}: no rules")
    return rules


def mask(name, value):
    if name == "tailnet-ip":
        return "100.x.x.x"
    return value[:4] + "…"


class Scanner:
    def __init__(self, patterns, allowlist):
        self.patterns, self.allowlist, self.hits = patterns, allowlist, 0

    def allowed(self, path, value):
        return any(fnmatch.fnmatchcase(path, glob) and regex.search(value)
                   for glob, regex in self.allowlist)

    def report(self, where, what):
        self.hits += 1
        print(f"LEAK {where}: {what}", file=sys.stderr)

    def scan_line(self, path, where, text):
        for name, regex in self.patterns:
            for match in regex.finditer(text):
                if not self.allowed(path, match.group(0)):
                    self.report(where, f"{name} {mask(name, match.group(0))}")


def git(*args, cwd=None):
    proc = subprocess.run(["git", *args], cwd=cwd, capture_output=True,
                          text=True, errors="replace")
    if proc.returncode:
        raise UsageError(f"git {' '.join(args)}: {proc.stderr.strip()}")
    return proc.stdout


def scan_tree(scanner):
    top = pathlib.Path(git("rev-parse", "--show-toplevel").strip())
    for rel in filter(None, git("ls-files", "-z", cwd=top).split("\0")):
        path = top / rel
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"\0" in data[:8192]:
            continue  # binary
        for lineno, text in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
            scanner.scan_line(rel, f"{rel}:{lineno}", text)


def resolve_range(spec):
    if "..." in spec:
        base, _, head = spec.partition("...")
        base = git("merge-base", base or "HEAD", head or "HEAD").strip()
    else:
        base, _, head = spec.partition("..")
    base, head = base or "HEAD", head or "HEAD"
    for rev in (base, head):
        git("rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}")
    return base, head


def scan_range(scanner, spec):
    base, head = resolve_range(spec)
    # Two-dot tree diff: covers merge commits, whose resolution `git log -p`
    # would hide. `+++ ` is a file header only between `diff --git` and the
    # first `@@`; inside a hunk it is an added line starting with `++ `.
    path, lineno, in_header = "?", 0, False
    for raw in git("diff", "--no-color", "--no-ext-diff", "-U0", base, head).splitlines():
        if raw.startswith("diff --git "):
            in_header = True
        elif in_header and raw.startswith("+++ "):
            path = raw[6:] if raw.startswith("+++ b/") else raw[4:]
        elif raw.startswith("@@"):
            in_header = False
            found = re.search(r"\+(\d+)", raw)
            lineno = int(found.group(1)) if found else 0
        elif not in_header and raw.startswith("+"):
            scanner.scan_line(path, f"{path}:{lineno}", raw[1:])
            lineno += 1
    log = git("log", "--format=%x00%h%x01%ae%x01%ce%x01%B", f"{base}..{head}")
    for commit in filter(None, log.split("\0")):
        sha, author_email, committer_email, body = commit.split("\x01", 3)
        for role, email in (("author", author_email), ("committer", committer_email)):
            if not IDENTITY_OK.search(email):
                scanner.report(f"commit {sha}", f"{role} email is not a noreply address")
        for lineno, text in enumerate(body.splitlines(), 1):
            scanner.scan_line("", f"commit {sha} message:{lineno}", text)


def run(argv):
    if len(argv) != 2:
        raise UsageError("expected one argument")
    scanner = Scanner(load_rules(HERE / "leak-patterns.txt"),
                      load_rules(HERE / "leak-allow.txt"))
    arg = argv[1]
    if arg == "--stdin":
        for lineno, text in enumerate(sys.stdin.read().splitlines(), 1):
            scanner.scan_line("", f"stdin:{lineno}", text)
    elif arg == "--tree":
        scan_tree(scanner)
    elif ".." in arg and not arg.startswith("-"):
        scan_range(scanner, arg)
    else:
        raise UsageError(f"unknown argument {arg!r}")
    if scanner.hits:
        print(f"leak-scan: {scanner.hits} hit(s). Fake test values must carry a marker "
              f"from tools/checks/leak-allow.txt (e.g. -LEAKCHECK-, -FAKE).", file=sys.stderr)
        return 1
    print("leak-scan: clean")
    return 0


def main(argv):
    try:
        return run(argv)
    except UsageError as err:
        print(f"leak-scan: {err}", file=sys.stderr)
        print(__doc__, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
