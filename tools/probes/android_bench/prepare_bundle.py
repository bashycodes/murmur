#!/usr/bin/env python3
"""Build the on-device benchmark fixture bundle (tools/probes/android_bench/README.md).

Public fixtures only, nothing personal:
  * en: the 40-clip LibriSpeech test-clean set (tools/probes/bench/asr_manifest_extended.json,
    CC BY 4.0), from $CACHE/audio (materialise with
    `$CACHE/venv/bin/python -I tools/probes/bench/fetch_audio.py --manifest asr_manifest_extended.json`).
  * he: the 49 FLEURS he_il dev clips of the 2026-10-30 laptop bench
    (tools/probes/prototypes/laptop_bench_2026_10_30/he_manifest.json, CC BY 4.0), extracted from
    google/fleurs data/he_il/audio/dev.tar.gz (153 MB, downloaded once into $CACHE/dl/fleurs-he/)
    and converted float32 -> PCM16 here (stdlib only).
  * cleanup: the 88 cases of the 2026-10-26 AI-cleanup bench (88-suite,
    ai_cleanup_bench_2026_10_26/results/v4c_cases.jsonl), reduced to id/app/category/input.

Output (default $CACHE/android-bench/bundle/, never committed): manifest.json, audio/{en,he}/*.wav,
cases.jsonl. About 25 MB with every clip; --en N / --he N keep the first N clips of a set.

usage: python3 -I tools/probes/android_bench/prepare_bundle.py [--out DIR] [--en N] [--he N] [--no-download]
"""
import argparse, array, json, os, pathlib, shutil, struct, sys, tarfile, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
PROBES = HERE.parent
CACHE = pathlib.Path(os.environ.get("CACHE", pathlib.Path.home() / ".cache/flow-clone"))
EN_MANIFEST = PROBES / "bench/asr_manifest_extended.json"
HE_MANIFEST = PROBES / "prototypes/laptop_bench_2026_10_30/he_manifest.json"
CASES = PROBES / "prototypes/ai_cleanup_bench_2026_10_26/results/v4c_cases.jsonl"
FLEURS_URL = "https://huggingface.co/datasets/google/fleurs/resolve/main/data/he_il/audio/dev.tar.gz"
RATE = 16_000


def wav_to_pcm16(data: bytes) -> bytes:
    """16 kHz mono WAV (PCM16 or float32) -> PCM16 samples."""
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    p, fmt, body = 12, None, None
    while p + 8 <= len(data):
        cid, size = data[p:p + 4], struct.unpack_from("<I", data, p + 4)[0]
        if cid == b"fmt ":
            fmt = struct.unpack_from("<HHIIHH", data, p + 8)
        elif cid == b"data":
            body = data[p + 8:p + 8 + size]
        p += 8 + size + (size & 1)
    if fmt is None or body is None:
        raise ValueError("no fmt/data chunk")
    tag, ch, rate, _, _, bits = fmt
    if ch != 1 or rate != RATE:
        raise ValueError(f"{ch} ch {rate} Hz; needs mono {RATE} Hz")
    if tag == 1 and bits == 16:
        return body
    if tag == 3 and bits == 32:
        f = array.array("f")
        f.frombytes(body[: len(body) // 4 * 4])
        if sys.byteorder != "little":
            f.byteswap()
        out = array.array("h", (max(-32768, min(32767, int(round(x * 32767)))) for x in f))
        if sys.byteorder != "little":
            out.byteswap()
        return out.tobytes()
    raise ValueError(f"unsupported WAV format {tag}/{bits}-bit")


def wav(pcm: bytes) -> bytes:
    return (b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " +
            struct.pack("<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16) + b"data" + struct.pack("<I", len(pcm)) + pcm)


def fleurs_tar(download: bool) -> pathlib.Path:
    p = CACHE / "dl/fleurs-he/dev.tar.gz"
    if not p.exists():
        if not download:
            sys.exit(f"missing {p} (run without --no-download)")
        p.parent.mkdir(parents=True, exist_ok=True)
        print(f"downloading {FLEURS_URL} (153 MB)", file=sys.stderr)
        tmp = p.with_suffix(".part")
        with urllib.request.urlopen(FLEURS_URL) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f, 1 << 20)
        tmp.rename(p)
    return p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=pathlib.Path, default=CACHE / "android-bench/bundle")
    ap.add_argument("--en", type=int, default=0, help="first N English clips (0 = all 40)")
    ap.add_argument("--he", type=int, default=0, help="first N Hebrew clips (0 = all 49)")
    ap.add_argument("--no-download", action="store_true")
    a = ap.parse_args()
    out = a.out
    if out.exists():
        shutil.rmtree(out)
    (out / "audio/en").mkdir(parents=True)
    (out / "audio/he").mkdir(parents=True)
    stt = []

    en = json.load(open(EN_MANIFEST))["utterances"]
    en = en[: a.en] if a.en else en
    missing = [u["id"] for u in en if not (CACHE / "audio" / f"{u['id']}.wav").exists()]
    if missing:
        sys.exit(f"{len(missing)} English clips missing from {CACHE / 'audio'} (e.g. {missing[0]}); "
                 "run tools/probes/bench/fetch_audio.py --manifest asr_manifest_extended.json")
    for u in en:
        pcm = wav_to_pcm16((CACHE / "audio" / f"{u['id']}.wav").read_bytes())
        (out / f"audio/en/{u['id']}.wav").write_bytes(wav(pcm))
        stt.append({"set": "en", "id": u["id"], "wav": f"audio/en/{u['id']}.wav", "dur_s": round(len(pcm) / 2 / RATE, 2)})

    he = json.load(open(HE_MANIFEST))["utterances"]
    he = he[: a.he] if a.he else he
    want = {f"dev/{u['id']}.wav": u["id"] for u in he}
    got = {}
    with tarfile.open(fleurs_tar(not a.no_download)) as t:
        for m in t:
            if m.name in want:
                got[want[m.name]] = wav_to_pcm16(t.extractfile(m).read())
    if len(got) != len(he):
        sys.exit(f"FLEURS tarball lacks {len(he) - len(got)} of the manifest's clips")
    for u in he:
        pcm = got[u["id"]]
        (out / f"audio/he/{u['id']}.wav").write_bytes(wav(pcm))
        stt.append({"set": "he", "id": u["id"], "wav": f"audio/he/{u['id']}.wav", "dur_s": round(len(pcm) / 2 / RATE, 2)})

    with open(CASES) as f, open(out / "cases.jsonl", "w") as o:
        n = 0
        for line in f:
            if line.strip():
                c = json.loads(line)
                o.write(json.dumps({k: c.get(k) or "" for k in ("id", "app_id", "app_category", "input")}, ensure_ascii=False) + "\n")
                n += 1

    json.dump({"version": 1, "about": "Flow Clone on-device bench bundle (public fixtures; see prepare_bundle.py)",
               "stt": stt, "cleanup": "cases.jsonl"}, open(out / "manifest.json", "w"), indent=1)
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    secs = {s: round(sum(c["dur_s"] for c in stt if c["set"] == s), 1) for s in ("en", "he")}
    print(f"{out}: {len(stt)} clips (en {secs['en']} s, he {secs['he']} s), {n} cleanup cases, {size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
