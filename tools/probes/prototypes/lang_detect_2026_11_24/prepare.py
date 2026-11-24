#!/usr/bin/env python3
"""Build the language-detection test set (public audio only, never committed).

Sets (16 kHz mono PCM16 WAVs + manifest.jsonl in OUT):
  he       FLEURS he_il dev (CC BY 4.0), every k-th clip of dev.tsv
  en       LibriSpeech test-clean (CC BY 4.0): the 40-clip extended bench set
           (tools/probes/bench/asr_manifest_extended.json) + 2 more per speaker
  he_short / en_short  the first 2.0 s of speech of some he / en clips (short dictations)
  mix_en_he / mix_he_en  an utterance of one language, 0.4 s pause, then the other;
           `expect` = the language with more speech
Every clip gets 0.3 s of quiet noise in front (the key press before speaking).

Usage:
  python3 -I prepare.py --fleurs-dir DIR --fleurs-tsv dev.tsv --librispeech-tar test-clean.tar.gz \
      [--out DIR] [--he N] [--en-extra-per-speaker 2]
OUT defaults to $LANG_DETECT_DATA or ~/.cache/murmur/lang-detect-2026-11-24.
"""
import argparse, json, os, pathlib, random, subprocess, sys, tarfile, tempfile, wave

import numpy as np

HERE = pathlib.Path(__file__).resolve().parent
EXT = HERE.parents[1] / "bench" / "asr_manifest_extended.json"
SR = 16000


def load(path: pathlib.Path) -> np.ndarray:
    """Any audio file -> float32 16 kHz mono via ffmpeg."""
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(SR), "-f", "f32le", "-"],
        check=True, capture_output=True).stdout
    return np.frombuffer(raw, dtype=np.float32).copy()


def lead(rng: random.Random) -> np.ndarray:
    n = int(0.3 * SR)
    return np.array([rng.uniform(-1, 1) * 0.001 for _ in range(n)], dtype=np.float32)


def speech_start(x: np.ndarray) -> int:
    """First 20 ms frame above -40 dBFS (prep only; the core has its own VAD)."""
    f = 320
    for i in range(0, len(x) - f, f):
        if np.sqrt(np.mean(x[i:i + f] ** 2)) > 0.01:
            return i
    return 0


def save(out: pathlib.Path, x: np.ndarray):
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR); w.writeframes(pcm.tobytes())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fleurs-dir", required=True)
    ap.add_argument("--fleurs-tsv", required=True)
    ap.add_argument("--librispeech-tar", required=True)
    ap.add_argument("--out", default=os.environ.get(
        "LANG_DETECT_DATA", str(pathlib.Path.home() / ".cache/murmur/lang-detect-2026-11-24")))
    ap.add_argument("--he", type=int, default=120)
    ap.add_argument("--en-extra-per-speaker", type=int, default=2)
    a = ap.parse_args()
    out = pathlib.Path(a.out); (out / "wav").mkdir(parents=True, exist_ok=True)
    rng = random.Random(20261124)
    rows = []

    def add(id_, set_, expect, x, ref, langs):
        p = out / "wav" / f"{id_}.wav"
        y = np.concatenate([lead(rng), x])
        save(p, y)
        rows.append(dict(id=id_, set=set_, expect=expect, path=str(p), ref=ref, langs=langs,
                         dur_s=round(len(y) / SR, 2)))

    # Hebrew: every k-th row of dev.tsv whose file exists.
    he_rows = []
    for line in open(a.fleurs_tsv, encoding="utf-8"):
        c = line.rstrip("\n").split("\t")
        if len(c) >= 3 and (pathlib.Path(a.fleurs_dir) / c[1]).is_file():
            he_rows.append((c[1], c[2]))
    k = max(1, len(he_rows) // a.he)
    he = [(f, r, load(pathlib.Path(a.fleurs_dir) / f)) for f, r in he_rows[::k][:a.he]]
    for f, r, x in he:
        add("he_" + f[:-4], "he", "he", x, r, ["he"])

    # English: extended bench ids + extra utterances per speaker (4-15 s).
    ext = {u["id"]: u["ref"] for u in json.load(open(EXT))["utterances"]}
    trans, flacs = {}, {}
    with tarfile.open(a.librispeech_tar) as t, tempfile.TemporaryDirectory() as td:
        members = t.getmembers()
        for m in members:
            if m.name.endswith(".trans.txt"):
                for l in t.extractfile(m).read().decode().splitlines():
                    i, txt = l.split(" ", 1); trans[i] = txt
            elif m.name.endswith(".flac"):
                flacs[pathlib.Path(m.name).stem] = m
        by_spk = {}
        for i in sorted(flacs):
            by_spk.setdefault(i.split("-")[0], []).append(i)
        want = set(ext)
        for spk in sorted({i.split("-")[0] for i in ext}):
            extra = [i for i in by_spk.get(spk, []) if i not in want]
            rng.shuffle(extra)
            want.update(extra[: a.en_extra_per_speaker * 3])
        en = []
        for i in sorted(want):
            m = flacs[i]
            t.extract(m, td, filter="data")
            x = load(pathlib.Path(td) / m.name)
            if i not in ext and not (4 * SR <= len(x) <= 15 * SR):
                continue
            en.append((i, trans[i], x))
        # Cap the extras per speaker.
        seen, kept = {}, []
        for i, r, x in en:
            spk = i.split("-")[0]
            if i in ext or seen.get(spk, 0) < a.en_extra_per_speaker:
                if i not in ext:
                    seen[spk] = seen.get(spk, 0) + 1
                kept.append((i, r, x))
        en = kept
    for i, r, x in en:
        add("en_" + i, "en", "en", x, r, ["en"])

    # Short: 2.0 s from the speech start.
    for set_, src in (("he_short", he[::3]), ("en_short", en[::3])):
        lang = set_[:2]
        for i, r, x in src[:40]:
            s = speech_start(x)
            add(f"{set_}_{pathlib.Path(i).stem}", set_, lang, x[max(0, s - 1600): s + 2 * SR], None, [lang])

    # Mixed: first language, 0.4 s pause, second language.
    gap = np.zeros(int(0.4 * SR), dtype=np.float32)
    he_s = sorted(he, key=lambda t: len(t[2]))
    en_s = sorted(en, key=lambda t: len(t[2]))
    for n in range(40):
        h, e = he_s[(n * 7) % len(he_s)], en_s[(n * 11) % len(en_s)]
        # Level-match the second clip to the first (FLEURS is quieter).
        def lvl(x):
            return np.sqrt(np.mean(x ** 2)) + 1e-9
        for first, second, set_ in ((e, h, "mix_en_he"), (h, e, "mix_he_en")):
            a1, a2 = first[2], second[2] * (lvl(first[2]) / lvl(second[2]))
            l1 = set_[4:6]; l2 = set_[7:9]
            expect = l1 if len(a1) >= len(a2) else l2
            add(f"{set_}_{n:02d}", set_, expect, np.concatenate([a1, gap, a2]), None, [l1, l2])

    with open(out / "manifest.jsonl", "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    counts = {}
    for r in rows:
        counts[r["set"]] = counts.get(r["set"], 0) + 1
    print(out / "manifest.jsonl", counts, file=sys.stderr)


if __name__ == "__main__":
    main()
