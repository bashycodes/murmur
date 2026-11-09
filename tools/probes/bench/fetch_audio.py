#!/usr/bin/env python3
"""Fetch the ASR test set listed in asr_manifest.json into $CACHE/audio as 16 kHz mono WAV.

Sources:
  * whisper.cpp samples/jfk.wav (public-domain JFK inaugural address excerpt; shipped in the
    whisper.cpp repo, which must already be cloned by build_engines.sh).
  * LibriSpeech test-clean (OpenSLR SLR12, CC BY 4.0, Panayotov et al. 2015). The 346 MB tarball is
    downloaded once into $CACHE/dl/ and only the manifest's utterances are extracted.

Run with: $CACHE/venv/bin/python -I tools/probes/bench/fetch_audio.py
(needs `soundfile` + `numpy`; the venv is created by README instructions in docs/model-benchmarks.md)

With --pick N it instead prints N candidate LibriSpeech utterances (5-20 s) to seed the manifest.
"""
import argparse, io, json, os, pathlib, shutil, subprocess, sys, tarfile, urllib.request

import numpy as np
import soundfile as sf

HERE = pathlib.Path(__file__).resolve().parent
CACHE = pathlib.Path(os.environ.get("CACHE", pathlib.Path.home() / ".cache/murmur"))
URL = "https://www.openslr.org/resources/12/test-clean.tar.gz"


def tarball() -> pathlib.Path:
    dl = CACHE / "dl"; dl.mkdir(parents=True, exist_ok=True)
    p = dl / "test-clean.tar.gz"
    if not p.exists():
        print("downloading", URL, file=sys.stderr)
        tmp = p.with_suffix(".part")
        with urllib.request.urlopen(URL) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f, 1 << 20)
        tmp.rename(p)
    return p



def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pick", type=int, default=0)
    ap.add_argument("--manifest", default="asr_manifest.json")
    a = ap.parse_args()
    man = json.loads((HERE / a.manifest).read_text())
    out = CACHE / "audio"; out.mkdir(parents=True, exist_ok=True)

    jfk_src = CACHE / "whisper.cpp/samples/jfk.wav"
    if not (out / "jfk.wav").exists():
        shutil.copy(jfk_src, out / "jfk.wav")

    want = {u["id"] for u in man["utterances"] if u["source"] == "librispeech-test-clean"}
    if a.pick == 0 and all((out / f"{w}.wav").exists() for w in want):
        print("audio present"); return

    tb = tarball()
    cands, trans = [], {}
    with tarfile.open(tb, "r:gz") as t:
        for m in t:
            if m.name.endswith(".trans.txt"):
                for line in t.extractfile(m).read().decode().splitlines():
                    k, _, v = line.partition(" ")
                    trans[k] = v
            elif m.name.endswith(".flac"):
                uid = pathlib.Path(m.name).stem
                if a.pick:
                    if len(cands) < 4000:
                        data, sr = sf.read(io.BytesIO(t.extractfile(m).read()))
                        d = len(data) / sr
                        if 5 <= d <= 20:
                            cands.append((uid, round(d, 2)))
                elif uid in want:
                    data, sr = sf.read(io.BytesIO(t.extractfile(m).read()), dtype="int16")
                    assert sr == 16000
                    sf.write(out / f"{uid}.wav", data, sr, subtype="PCM_16")
                    print("wrote", uid, file=sys.stderr)
    if a.pick:
        # one utterance per speaker for diversity, spread over durations
        seen, picked = set(), []
        for uid, d in cands:
            spk = uid.split("-")[0]
            if spk in seen: continue
            seen.add(spk); picked.append({"id": uid, "dur_s": d, "ref": trans[uid]})
        step = max(1, len(picked) // a.pick)
        print(json.dumps(picked[::step][: a.pick], indent=1))


if __name__ == "__main__":
    main()
