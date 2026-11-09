#!/usr/bin/env python3
"""The licence file inside each colleague-bundle model zip (D80).

    model_notice.py FOLDER...                 # MODEL-LICENSES.txt on stdout
    model_notice.py --check-catalog CATALOG_RS

FOLDER: the model folders in the zip (column 1 of models_manifest.py).
Prints each model's licence and attribution from tools/notice/models.tsv,
the notices its licence asks for ("Built with Llama", NVIDIA's notice),
then the full texts (third_party/licenses/), each once. A folder without a
row is listed as "licence unconfirmed" rather than given one.

--check-catalog: every catalogue entry of model_download.rs has a row whose
licence equals the entry's `license`, and every row names texts that exist.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TABLE = ROOT / "tools/notice/models.tsv"
TEXTS = ROOT / "third_party/licenses"
NOTICES = {
    "Llama 3.2 Community License": "Built with Llama. Llama 3.2 is licensed under the Llama 3.2 Community "
                                   "License, Copyright (c) Meta Platforms, Inc. All Rights Reserved.",
    "NVIDIA Open Model License": "Licensed by NVIDIA Corporation under the NVIDIA Open Model License.",
}


def table():
    rows = {}
    for line in TABLE.read_text(encoding="utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            folder, texts, lic = line.split("\t")
            rows[folder] = (texts.split(","), lic)
    return rows


def notice(folders):
    rows = table()
    out = ["Licences of the models in this archive", "",
           "Each model is the work of its authors, under the licence named here (from",
           "its model card). The full texts follow.", ""]
    texts, extra = [], []
    for f in folders:
        if f not in rows:
            out.append(f"- {f}: licence unconfirmed; see the model's page before use")
            continue
        names, lic = rows[f]
        out.append(f"- {f}: {lic}")
        for key, text in NOTICES.items():
            if lic.startswith(key) and text not in extra:
                extra.append(text)
        texts += [n for n in names if n not in texts]
    if extra:
        out += ["", "Notices:"] + [f"  {t}" for t in extra]
    for n in texts:
        out += ["", "=" * 78, n, "=" * 78, "", (TEXTS / n).read_text(encoding="utf-8").rstrip()]
    return "\n".join(out) + "\n"


def check_catalog(rs):
    s = pathlib.Path(rs).read_text(encoding="utf-8")
    consts = dict(re.findall(r'pub const (\w+): &str = "([^"]*)";', s))
    cat = s[s.index("pub const CATALOG"):]
    cat = cat[:cat.index("\n];")]
    rows, bad = table(), []
    for block in cat.split("CatalogEntry {")[1:]:
        m = re.search(r'\bid: (?:"([^"]+)"|(\w+))', block)
        cid = m.group(1) or consts[m.group(2)]
        lic = re.search(r'\blicense: (?:Some\("([^"]*)"\)|None)', block).group(1)
        if cid not in rows:
            bad.append(f"{cid}: no row in {TABLE.name}")
        elif rows[cid][1] != lic:
            bad.append(f"{cid}: catalogue says {lic!r}, {TABLE.name} {rows[cid][1]!r}")
    bad += [f"{f}: no text {n}" for f, (names, _) in rows.items() for n in names if not (TEXTS / n).is_file()]
    if bad:
        sys.exit("model_notice.py: " + "; ".join(bad))


if __name__ == "__main__":
    if sys.argv[1:2] == ["--check-catalog"]:
        check_catalog(sys.argv[2])
    else:
        sys.stdout.write(notice(sys.argv[1:]))
