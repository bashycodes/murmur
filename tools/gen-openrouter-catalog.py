#!/usr/bin/env python3
"""Generate crates/flow-cloud/data/catalog-baked.json (D28 §2.2).

The baked catalogue is flow-cloud's offline fallback: used when there is no
cache file and no network. It is a *trimmed copy of the raw OpenRouter
JSON* (not a pre-parsed catalogue), so the Rust parser that reads live
responses (`flow_cloud::catalog::Catalog::from_api`) also reads this file:
one parser, no Python/Rust drift.

Input: the dated snapshots in docs/proposals/data/
  openrouter-models-<date>.json                (GET /api/v1/models)
  openrouter-models-transcription-<date>.json  (GET /api/v1/models?output_modalities=transcription)
  openrouter-endpoints-zdr-<date>.json         (GET /api/v1/endpoints/zdr)

Selection: every transcription model (path A), every non-batch, non-router
audio-input chat model with a ZDR endpoint (path B), the recommended
cleanup models plus the cheapest ZDR non-reasoning text models (up to
CLEANUP_EXTRA). Fields are trimmed to what the parser reads.

Usage: tools/gen-openrouter-catalog.py [DATE]   (default: newest snapshot)
"""

import glob
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(ROOT, "docs", "proposals", "data")
OUT = os.path.join(ROOT, "crates", "flow-cloud", "data", "catalog-baked.json")

# Keep in sync with flow_cloud::catalog::RECOMMENDED_* (a Rust test checks
# every one of them is in the baked file).
RECOMMENDED_STT = [
    "openai/whisper-large-v3-turbo",
    "elevenlabs/scribe-v2",
    "mistralai/voxtral-mini-transcribe-3-0",
    "nvidia/parakeet-tdt-0.6b-v3",
    "deepgram/nova-3",
]
RECOMMENDED_CLEANUP = [
    "meta-llama/llama-3.3-70b-instruct",
    "meta-llama/llama-3.1-8b-instruct",
    "mistralai/mistral-small-3.2-24b-instruct",
    "google/gemini-3.1-flash-lite",
    "openai/gpt-oss-20b",
]
CLEANUP_EXTRA = 15

MODEL_KEYS = ["id", "name", "context_length", "pricing", "supported_parameters", "reasoning"]
PRICE_KEYS = ["prompt", "completion", "request", "audio"]


def newest_date():
    dates = []
    for p in glob.glob(os.path.join(DATA, "openrouter-models-transcription-*.json")):
        m = re.search(r"(\d{4}-\d{2}-\d{2})\.json$", p)
        if m:
            dates.append(m.group(1))
    if not dates:
        sys.exit("no snapshots in " + DATA)
    return max(dates)


def load(name, date):
    with open(os.path.join(DATA, f"{name}-{date}.json"), encoding="utf-8") as f:
        return json.load(f)


def trim_price(p):
    return {k: v for k, v in (p or {}).items() if k in PRICE_KEYS}


def trim_model(m):
    out = {k: m[k] for k in MODEL_KEYS if k in m and m[k] is not None}
    out["pricing"] = trim_price(m.get("pricing"))
    arch = m.get("architecture") or {}
    out["architecture"] = {
        "input_modalities": arch.get("input_modalities") or [],
        "output_modalities": arch.get("output_modalities") or [],
    }
    mct = (m.get("top_provider") or {}).get("max_completion_tokens")
    if mct is not None:
        out["top_provider"] = {"max_completion_tokens": mct}
    if "reasoning" in out:
        r = out["reasoning"]
        out["reasoning"] = {k: r[k] for k in ("mandatory", "supported_efforts") if k in r}
    return out


def is_router(m):
    p = m.get("pricing") or {}
    return m["id"].startswith("openrouter/") or any(str(p.get(k, "0")).startswith("-") for k in PRICE_KEYS)


def price(m, k):
    try:
        return float((m.get("pricing") or {}).get(k, "inf"))
    except ValueError:
        return float("inf")


def main():
    date = sys.argv[1] if len(sys.argv) > 1 else newest_date()
    models = load("openrouter-models", date)["data"]
    trans = load("openrouter-models-transcription", date)["data"]
    zdr = load("openrouter-endpoints-zdr", date)["data"]
    zdr_ids = {e["model_id"] for e in zdr}

    keep = {}
    for m in trans:
        keep[m["id"]] = m
    for m in models:
        arch = m.get("architecture") or {}
        ok = not m["id"].endswith(":batch") and not is_router(m)
        if ok and "audio" in (arch.get("input_modalities") or []) and m["id"] in zdr_ids:
            keep[m["id"]] = m
    by_id = {m["id"]: m for m in models}
    for i in RECOMMENDED_CLEANUP:
        if i not in by_id:
            sys.exit(f"recommended cleanup model {i} not in the snapshot")
        keep[i] = by_id[i]
    for i in RECOMMENDED_STT:
        if i not in keep:
            sys.exit(f"recommended STT model {i} not in the snapshot")
    extra = [
        m
        for m in models
        if m["id"] not in keep
        and m["id"] in zdr_ids
        and not m["id"].endswith(":batch")
        and not m["id"].endswith(":free")
        and not is_router(m)
        and "text" in (m.get("architecture") or {}).get("input_modalities", [])
        and (m.get("architecture") or {}).get("output_modalities") == ["text"]
        and not (m.get("reasoning") or {}).get("mandatory", False)
    ]
    extra.sort(key=lambda m: (price(m, "prompt") + 2 * price(m, "completion"), m["id"]))
    for m in extra[:CLEANUP_EXTRA]:
        keep[m["id"]] = m

    ids = sorted(keep)
    out = {
        "snapshot_date": date,
        "source_note": "trimmed from docs/proposals/data/*-" + date + ".json by tools/gen-openrouter-catalog.py",
        "models": {"data": [trim_model(keep[i]) for i in ids]},
        "zdr": {
            "data": [
                {"model_id": e["model_id"], "tag": e.get("tag", ""), "pricing": trim_price(e.get("pricing"))}
                for e in sorted(zdr, key=lambda e: (e["model_id"], e.get("tag", "")))
                if e["model_id"] in keep
            ]
        },
    }
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")
    n_stt = sum(1 for i in ids if "transcription" in (keep[i].get("architecture") or {}).get("output_modalities", []))
    print(f"{os.path.relpath(OUT, ROOT)}: {len(ids)} models ({n_stt} transcription), {len(out['zdr']['data'])} ZDR endpoints, snapshot {date}")


if __name__ == "__main__":
    main()
