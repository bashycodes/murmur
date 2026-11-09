#!/usr/bin/env python3
"""Regenerate NOTICE's Rust crate section and THIRD-PARTY-LICENSES (D80).

    python3 -I tools/notice/crate-licences.py           # rewrite both
    python3 -I tools/notice/crate-licences.py --check   # fail if either is stale

The crates are the ones linked into what we ship: flow-server, flow-ffi
(Android) and flow-llm from the root workspace, flow-windows from
apps/flow-windows, for the platforms they ship on, with every feature on
(a superset: CUDA/Vulkan builds included). Dev-dependencies and build
scripts' own dependencies are left out; proc-macros are kept. Our own path
crates are left out. Runs `cargo metadata --offline`, so the registry cache
must hold every crate (`cargo fetch --locked` in both workspaces fills it).

THIRD-PARTY-LICENSES holds the full texts: each crate's own licence and
notice files (for the -sys crates also those of the C/C++ sources they
vendor: llama.cpp, whisper.cpp, SQLite), then third_party/licenses/ (the
native libraries, installer, Android libraries and models; refreshed by
tools/notice/fetch-texts.sh), the font licences and the standard SPDX
texts for crates that ship no file of their own.
"""
import collections
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import textwrap

ROOT = pathlib.Path(__file__).resolve().parents[2]
NOTICE = ROOT / "NOTICE"
BEGIN = "--- BEGIN GENERATED: Rust crates (tools/notice/crate-licences.py) ---"
END = "--- END GENERATED: Rust crates ---"
APPENDIX = ROOT / "THIRD-PARTY-LICENSES"
TEXTS = ROOT / "third_party/licenses"
FONTS = ROOT / "apps/flow-windows/ui/dist/fonts"
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)([-_.][^/]*)?$", re.I)
NOT_TEXT = {".cmake", ".rs", ".py", ".c", ".h", ".cpp", ".toml", ".json", ".yml", ".yaml", ".html", ".sh"}
SYS_CRATES = {"llama-cpp-sys-2", "whisper-rs-sys", "libsqlite3-sys"}
# (workspace dir, shipped root crates, target platforms)
SHIPPED = [
    (ROOT, ["flow-server", "flow-ffi", "flow-llm"],
     ["x86_64-unknown-linux-gnu", "aarch64-linux-android", "x86_64-pc-windows-msvc"]),
    (ROOT / "apps/flow-windows", ["flow-windows"], ["x86_64-pc-windows-msvc"]),
]


def metadata(ws, target):
    out = subprocess.run(
        ["cargo", "metadata", "--offline", "--all-features", "--format-version", "1", "--filter-platform", target],
        cwd=ws, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def shipped_packages():
    found = {}
    for ws, roots, targets in SHIPPED:
        for target in targets:
            m = metadata(ws, target)
            pkgs = {p["id"]: p for p in m["packages"]}
            nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
            todo = [i for i, p in pkgs.items() if p["name"] in roots and p["source"] is None]
            seen = set()
            while todo:
                i = todo.pop()
                if i in seen:
                    continue
                seen.add(i)
                for d in nodes[i]["deps"]:
                    if any(k["kind"] is None for k in d["dep_kinds"]):
                        todo.append(d["pkg"])
            for i in seen:
                p = pkgs[i]
                if p["source"] is not None:
                    found[(p["name"], p["version"])] = p
    return found.values()


def normalise(lic):
    """SPDX-ish: `A/B` -> `A OR B`; a plain OR (or AND) list in a fixed order."""
    lic = re.sub(r"\s*/\s*", " OR ", (lic or "UNKNOWN").strip())
    if "(" not in lic and " WITH " not in lic:
        for op in (" OR ", " AND "):
            if op in lic and (" AND " if op == " OR " else " OR ") not in lic:
                lic = op.join(sorted(t.strip() for t in lic.split(op)))
    return lic


def section(pkgs):
    groups = collections.defaultdict(set)
    for p in pkgs:
        groups[normalise(p.get("license") or ("see " + p["license_file"] if p.get("license_file") else None))].add(p["name"])
    total = sum(len(v) for v in groups.values())
    lines = [BEGIN, "", f"{total} crates from crates.io (https://crates.io/crates/<name>; their licence",
             "texts are in THIRD-PARTY-LICENSES, Part 1), grouped by licence:"]
    for lic, names in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        lines += ["", f"{lic} ({len(names)}):"]
        lines += textwrap.wrap(", ".join(sorted(names)), 76, initial_indent="  ", subsequent_indent="  ",
                               break_on_hyphens=False)
    return "\n".join(lines + ["", END])


def licence_files(pkg):
    """A crate's licence/notice files; for SYS_CRATES also those of vendored sources."""
    top = pathlib.Path(pkg["manifest_path"]).parent
    depth = 5 if pkg["name"] in SYS_CRATES else 1
    out = []
    for f in sorted(top.rglob("*")):
        rel = f.relative_to(top)
        if (len(rel.parts) <= depth and f.is_file() and LICENCE_FILE.match(f.name)
                and f.suffix.lower() not in NOT_TEXT
                and not {"tests", "test", "examples", "docs"} & set(rel.parts[:-1])):
            out.append((str(rel), f.read_text(encoding="utf-8", errors="replace")))
    return out


def spdx_ids(lic):
    return sorted(set(re.findall(r"[A-Za-z0-9.+-]+", lic)) - {"OR", "AND", "WITH"})


def rule(title):
    return f"\n{'=' * 78}\n{title}\n{'=' * 78}\n"


def appendix(pkgs):
    seen = {}  # normalised text hash -> where it was first printed
    parts = ["THIRD-PARTY LICENCES\n\nFull licence and notice texts of the third-party software and models that\n"
             "Murmur ships or downloads; NOTICE lists them. Generated by\n"
             "tools/notice/crate-licences.py; do not edit.\n"]

    def emit(where, text):
        h = hashlib.sha256(" ".join(text.split()).encode()).hexdigest()
        if h in seen:
            parts.append(f"\n--- {where}: same text as {seen[h]} ---\n")
            return
        seen[h] = where
        parts.append(f"\n--- {where} ---\n\n{text.rstrip()}\n")

    parts.append(rule("Part 1. Rust crates (crates.io)"))
    for p in sorted(pkgs, key=lambda p: (p["name"], p["version"])):
        lic = normalise(p.get("license"))
        files = licence_files(p)
        parts.append(f"\n## {p['name']} {p['version']} ({lic})\n")
        if not files:
            parts.append("Ships no licence file; the standard text of "
                         + ", ".join(spdx_ids(lic)) + " is in Part 4.\n")
        for rel, text in files:
            emit(f"{p['name']} {p['version']}/{rel}", text)
    parts.append(rule("Part 2. Native libraries, installer, Android libraries, models"))
    for f in sorted(TEXTS.rglob("*")):
        rel = f.relative_to(TEXTS)
        if f.is_file() and rel.parts[0] != "spdx" and f.name != "SHA256SUMS":
            emit(str(rel), f.read_text(encoding="utf-8"))
    parts.append(rule("Part 3. Fonts (SIL Open Font License 1.1)"))
    for f in sorted(FONTS.glob("*-OFL.txt")):
        emit(f"fonts/{f.name}", f.read_text(encoding="utf-8"))
    parts.append(rule("Part 4. Standard licence texts (SPDX license-list-data 3.25.0)"))
    for f in sorted((TEXTS / "spdx").glob("*.txt")):
        emit(f"spdx/{f.name}", f.read_text(encoding="utf-8"))
    return "".join(parts)


def main():
    pkgs = list(shipped_packages())
    text = NOTICE.read_text(encoding="utf-8")
    a, b = text.index(BEGIN), text.index(END) + len(END)
    new = text[:a] + section(pkgs) + text[b:]
    old_app = APPENDIX.read_text(encoding="utf-8") if APPENDIX.exists() else ""
    new_app = appendix(pkgs)
    missing = {i for p in pkgs for i in spdx_ids(normalise(p.get("license")))
               if not licence_files(p) and not (TEXTS / "spdx" / f"{i}.txt").exists()}
    if missing:
        sys.exit(f"no standard text for {sorted(missing)}: add them to tools/notice/fetch-texts.sh")
    if "--check" in sys.argv[1:]:
        stale = [n for n, o, w in (("NOTICE (Rust crate section)", text, new),
                                   ("THIRD-PARTY-LICENSES", old_app, new_app)) if o != w]
        if stale:
            sys.exit(f"stale: {', '.join(stale)}; run python3 -I tools/notice/crate-licences.py")
        print("NOTICE crate section and THIRD-PARTY-LICENSES current")
        return
    NOTICE.write_text(new, encoding="utf-8")
    APPENDIX.write_text(new_app, encoding="utf-8")


if __name__ == "__main__":
    main()
