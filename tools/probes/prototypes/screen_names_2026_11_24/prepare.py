#!/usr/bin/env python3
"""Build the on-screen-names probe set (synthetic or public audio only, never committed).

Rows (16 kHz mono PCM16 WAVs + manifest.jsonl in OUT):
  names  one clip per `said` sentence of the English screens in
         crates/murmur-core/fixtures/screen-names/chat-screens.json (20), spoken by
         espeak-ng (offline TTS; every name and sentence is invented). `name` is the
         planted name the sentence carries, `screen` its chat screen.
  bench  the 40-clip LibriSpeech test-clean bench set (tools/probes/bench/
         asr_manifest_extended.json), reused from the lang-detect probe's WAVs
         (--bench-manifest, rows `en_<id>`); each gets an English screen in turn, so
         the WER shows what unrelated screen words cost.

Usage:
  python3 -I prepare.py --espeak ESPEAK_NG [--espeak-path DATA_DIR] \\
      --bench-manifest ~/.cache/murmur/lang-detect-2026-11-24/manifest.jsonl [--out DIR]
OUT defaults to $SCREEN_NAMES_DATA or ~/.cache/murmur/screen-names-2026-11-24.
"""
import argparse, json, os, pathlib, subprocess, tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SCREENS = ROOT / "crates/murmur-core/fixtures/screen-names/chat-screens.json"
BENCH = ROOT / "tools/probes/bench/asr_manifest_extended.json"
# Two espeak voices so the set is not one speaker.
VOICES = ["en-us", "en-gb"]


def tts(espeak, data, text, voice, out):
    with tempfile.TemporaryDirectory() as d:
        raw = pathlib.Path(d) / "raw.wav"
        cmd = [espeak, "-v", voice, "-s", "150", "-w", str(raw), text]
        if data:
            cmd[1:1] = [f"--path={data}"]
        subprocess.run(cmd, check=True, capture_output=True)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(raw), "-af",
                        "adelay=300:all=1,apad=pad_dur=0.3", "-ac", "1", "-ar", "16000",
                        "-c:a", "pcm_s16le", str(out)], check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--espeak", required=True, help="espeak-ng binary")
    ap.add_argument("--espeak-path", help="dir holding espeak-ng-data (--path)")
    ap.add_argument("--bench-manifest", required=True)
    ap.add_argument("--out", default=os.environ.get(
        "SCREEN_NAMES_DATA", str(pathlib.Path.home() / ".cache/murmur/screen-names-2026-11-24")))
    a = ap.parse_args()
    out = pathlib.Path(a.out); (out / "wav").mkdir(parents=True, exist_ok=True)
    screens = json.load(open(SCREENS, encoding="utf-8"))["screens"]
    english = [s for s in screens if s["said"]]
    rows = []
    k = 0
    for s in english:
        for said in s["said"]:
            name = next(n for n in s["names"] if n.lower() in said.lower())
            p = out / "wav" / f"names_{k:02d}.wav"
            tts(a.espeak, a.espeak_path, said, VOICES[k % len(VOICES)], p)
            rows.append(dict(id=f"names_{k:02d}", set="names", path=str(p), ref=said,
                             name=name, screen=s["id"]))
            k += 1
    have = {r["id"]: r for r in map(json.loads, open(a.bench_manifest, encoding="utf-8"))}
    bench = json.load(open(BENCH, encoding="utf-8"))["utterances"]
    for i, u in enumerate(bench):
        r = have[f"en_{u['id']}"]
        rows.append(dict(id=u["id"], set="bench", path=r["path"], ref=u["ref"],
                         screen=english[i % len(english)]["id"]))
    with open(out / "manifest.jsonl", "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{k} name clips, {len(bench)} bench clips -> {out / 'manifest.jsonl'}")


if __name__ == "__main__":
    main()
