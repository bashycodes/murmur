#!/usr/bin/env python3
"""Regenerate NOTICE's Rust crate section from `cargo metadata`.

    python3 -I tools/notice/crate-licences.py           # rewrite NOTICE
    python3 -I tools/notice/crate-licences.py --check   # fail if NOTICE is stale

The crates are the ones linked into what we ship: flow-server, flow-ffi
(Android) and flow-llm from the root workspace, flow-windows from
apps/flow-windows, for the platforms they ship on, with every feature on
(a superset: CUDA/Vulkan builds included). Dev-dependencies and build
scripts' own dependencies are left out; proc-macros are kept. Our own path
crates are left out. Runs `cargo metadata --offline`, so the registry cache
must hold every crate (a normal `cargo build` fills it).
"""
import collections
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


def section():
    groups = collections.defaultdict(set)
    for p in shipped_packages():
        groups[normalise(p.get("license") or ("see " + p["license_file"] if p.get("license_file") else None))].add(p["name"])
    total = sum(len(v) for v in groups.values())
    lines = [BEGIN, "", f"{total} crates from crates.io (https://crates.io/crates/<name>; each crate's",
             "source there carries its licence text), grouped by licence:"]
    for lic, names in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        lines += ["", f"{lic} ({len(names)}):"]
        lines += textwrap.wrap(", ".join(sorted(names)), 76, initial_indent="  ", subsequent_indent="  ",
                               break_on_hyphens=False)
    return "\n".join(lines + ["", END])


def main():
    text = NOTICE.read_text(encoding="utf-8")
    a, b = text.index(BEGIN), text.index(END) + len(END)
    new = text[:a] + section() + text[b:]
    if "--check" in sys.argv[1:]:
        if new != text:
            sys.exit("NOTICE: the Rust crate section is stale; run python3 -I tools/notice/crate-licences.py")
        print("NOTICE crate section current")
        return
    NOTICE.write_text(new, encoding="utf-8")


if __name__ == "__main__":
    main()
