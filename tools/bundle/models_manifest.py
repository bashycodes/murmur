#!/usr/bin/env python3
"""Pinned model list for tools/colleague-bundle.sh, checked against the apps.

    models_manifest.py LIST CATALOG_RS CORE_ADAPTER_RS

LIST is docs/air-gapped-models.txt (server library layout: one line per
file, `folder kind engine file bytes sha256 url title`). CATALOG_RS is
apps/flow-windows/src/model_download.rs, the apps' catalogue with the pins
(Android pins the same; flow_protocol's test keeps them in step), and
CORE_ADAPTER_RS the file its `crate::core_adapter::X` folder names live in.
All three are read from the bundle's source commit, so a branch that adds
models (the notetaker's speaker models, D61) brings them without a change
here.

Prints one tab-separated line per file:
    folder kind engine file bytes sha256 url title
Every LIST line must name a file the catalogue pins with the same size,
SHA-256 and URL, or this exits 1. Catalogue entries with engine
`Diarization` (not a server library kind) are added with kind `diarizer`;
any other catalogue file missing from LIST is an error too, so the list
cannot fall behind the apps unnoticed.
"""
import re
import sys

MACRO = re.compile(r'macro_rules!\s*(\w+)\s*\{\s*\(\)\s*=>\s*\{\s*"([^"]+)"')
FILE = re.compile(r'model_file!\(\s*(\w+)!\(\)\s*,\s*"([^"]+)"\s*,\s*([\d_]+)\s*,\s*"([0-9a-f]{64})"\s*\)')
CONST = re.compile(r'(?:pub(?:\(crate\))?\s+)?const\s+(\w+)\s*:\s*&str\s*=\s*"([^"]*)"')


def die(msg):
    print(f"models_manifest.py: {msg}", file=sys.stderr)
    sys.exit(1)


def field(block, name, consts):
    m = re.search(rf'\b{name}:\s*(?:"([^"]*)"|([\w:]+))', block)
    if not m:
        return None
    if m.group(1) is not None:
        return m.group(1)
    key = m.group(2).split("::")[-1]
    if key not in consts:
        die(f"cannot resolve {m.group(2)}")
    return consts[key]


def catalogue(rs, adapter):
    bases = dict(MACRO.findall(rs))
    consts = dict(CONST.findall(adapter))
    consts.update(CONST.findall(rs))
    files = {}
    for base, name, size, sha in FILE.findall(rs):
        if base not in bases:
            die(f"unknown URL base {base}!()")
        files[sha] = (name, int(size.replace("_", "")), bases[base] + name)
    start = rs.find("pub const CATALOG")
    if start < 0:
        die("no CATALOG in the catalogue file")
    end = rs.find("\n];", start)
    entries = []
    for block in rs[start:end].split("CatalogEntry {")[1:]:
        engine = re.search(r'engine:\s*ModelEngine::(\w+)', block)
        files_part = block.split("files:", 1)[-1]
        entries.append({
            "id": field(block, "id", consts),
            "title": field(block, "title", consts),
            "dest": field(block, "dest", consts),
            "engine": engine.group(1) if engine else None,
            "files": [sha for _, _, _, sha in FILE.findall(files_part)],
        })
    return files, entries


def main(argv):
    if len(argv) != 4:
        die("usage: models_manifest.py LIST CATALOG_RS CORE_ADAPTER_RS")
    with open(argv[1], encoding="utf-8") as f:
        lines = f.read().splitlines()
    with open(argv[2], encoding="utf-8") as f:
        rs = f.read()
    with open(argv[3], encoding="utf-8") as f:
        adapter = f.read()
    files, entries = catalogue(rs, adapter)
    out, seen = [], set()
    for ln in lines:
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        parts = ln.split(None, 7)
        if len(parts) != 8:
            die(f"bad list line: {ln}")
        folder, kind, engine, name, size, sha, url, title = parts
        pin = files.get(sha)
        if pin is None:
            die(f"{folder}/{name}: SHA-256 {sha} is not in the catalogue")
        if pin != (name, int(size), url):
            die(f"{folder}/{name}: list says {name} {size} {url}, catalogue {pin}")
        seen.add(sha)
        out.append((folder, kind, engine, name, size, sha, url, title))
    for e in entries:
        if e["engine"] != "Diarization":
            continue
        for sha in e["files"]:
            if sha in seen:
                continue
            name, size, url = files[sha]
            seen.add(sha)
            out.append((e["dest"], "diarizer", "sherpa_diarization", name, str(size), sha, url, e["title"]))
    missing = sorted(files[s][0] for s in files if s not in seen)
    if missing:
        die("catalogue files missing from the list: " + ", ".join(missing))
    for row in out:
        print("\t".join(row))


if __name__ == "__main__":
    main(sys.argv)
