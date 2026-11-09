#!/usr/bin/env python3
"""Probe (2026-10-23, phone quality regression diag): does whisper multilingual
base-q5_1 with language "auto" mis-detect / garble short English clips, vs
--lang en and vs the English-only base.en model?

Clips: the bench core set (jfk + 12 LibriSpeech), full + first 2 s / 3 s crops,
clean + with white noise at ~15 dB SNR (phone-ish). Runs flow-cli transcribe. Env: SNR_DB (default 15), CFGS (subset), OUT.
Writes JSONL to $OUT (default ~/.cache/flow-clone/diag-2026-10-23/lang_auto.jsonl)
and prints a summary table. Not promoted; findings in
docs/findings/2026-10-23-phone-quality-regression.md.

usage: python3 -I lang_auto_short_clips_2026_10_23.py FLOW_CLI MANIFEST WER_PY AUDIO_DIR JFK_WAV
"""
import json, math, os, random, struct, subprocess, sys, tempfile, wave, importlib.util

cli, manifest, wer_py, audio_dir, jfk = sys.argv[1:6]
M = os.path.expanduser("~/.cache/flow-clone/models")
CONFIGS = {
    "base-q5_1/auto": ["--model", f"{M}/ggml-base-q5_1.bin"],
    "base-q5_1/en": ["--model", f"{M}/ggml-base-q5_1.bin", "--lang", "en"],
    "base.en": ["--model", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "models", "ggml-base.en.bin")],
}
out_path = os.environ.get("OUT", os.path.expanduser("~/.cache/flow-clone/diag-2026-10-23/lang_auto.jsonl"))
spec = importlib.util.spec_from_file_location("wer", wer_py); W = importlib.util.module_from_spec(spec); spec.loader.exec_module(W)
wer_fn = getattr(W, "wer", None)

def read(p):
    with wave.open(p) as w:
        assert w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2, p
        return list(struct.unpack(f"<{w.getnframes()}h", w.readframes(w.getnframes())))

def write(p, s):
    with wave.open(p, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
        w.writeframes(struct.pack(f"<{len(s)}h", *[max(-32768, min(32767, int(v))) for v in s]))

SNR = float(os.environ.get("SNR_DB", "15"))
ONLY = os.environ.get("CFGS")  # comma list of config names to run
if ONLY:
    CONFIGS = {k: v for k, v in CONFIGS.items() if k in ONLY.split(",")}

def noisy(s, snr_db=None, seed=1):
    snr_db = SNR if snr_db is None else snr_db
    r = random.Random(seed); p = sum(v * v for v in s) / len(s)
    sd = math.sqrt(p / (10 ** (snr_db / 10)))
    return [v + r.gauss(0, sd) for v in s]

utts = json.load(open(manifest))["utterances"]
tmp = tempfile.mkdtemp(prefix="langauto-")
rows = []
with open(out_path, "w") as out:
    for u in utts:
        src = jfk if u["id"] == "jfk" else os.path.join(audio_dir, u["id"] + ".wav")
        s = read(src)
        for crop in ("full", "3s", "2s"):
            c = s if crop == "full" else s[: 16000 * int(crop[0])]
            for noise in ("clean", "snr"):
                if crop == "full" and noise == "snr":
                    continue
                c2 = c if noise == "clean" else noisy(c)
                wav = os.path.join(tmp, f"{u['id']}-{crop}-{noise}.wav"); write(wav, c2)
                for name, args in CONFIGS.items():
                    r = subprocess.run([cli, "transcribe", *args, "--platform", "android", "--threads", "4", "--json", wav],
                                       capture_output=True, text=True)
                    try:
                        j = json.loads(r.stdout)
                    except Exception:
                        j = {"asr_text": "", "language": None, "error": r.stderr[-300:]}
                    hyp = j.get("asr_text", "")
                    row = {"id": u["id"], "crop": crop, "noise": noise, "cfg": name, "lang": j.get("language"), "hyp": hyp}
                    if crop == "full" and wer_fn:
                        row["wer"] = wer_fn(u["ref"], hyp)
                    out.write(json.dumps(row) + "\n"); out.flush(); rows.append(row)
                    print(name, u["id"], crop, noise, row["lang"], hyp[:70], flush=True)

print("\n== summary: non-English detections / total, mean WER (full clips)")
for name in CONFIGS:
    for crop in ("full", "3s", "2s"):
        for noise in ("clean", "snr"):
            rs = [r for r in rows if r["cfg"] == name and r["crop"] == crop and r["noise"] == noise]
            if not rs: continue
            nonen = sum(1 for r in rs if r["lang"] not in ("en", None))
            w = [r["wer"] for r in rs if "wer" in r]
            print(f"{name:16} {crop:4} {noise:5} non-en {nonen}/{len(rs)}" + (f"  WER {sum(w)/len(w):.3f}" if w else ""))
