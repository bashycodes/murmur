#!/usr/bin/env python3
"""STT WER + latency on the Windows PC (dated probe; copy of laptop_bench_2026_10_30/asr_win.py
with a `whisper_vk` engine added 2026-10-31; runs ON the PC).
  whisper_vk  whisper-cli.exe b5454 with llama.cpp b12005's ggml*.dll set (Vulkan backend)
              in BIN/whisper-vk2; same flags as whisper (GPU is whisper-cli's default)

Runs official release binaries one process per clip, at below-normal priority:
  whisper   whisper-cli.exe (whisper.cpp release, CPU)  -l LANG -bs 1 -bo 1 -nt [-ac N]
  offline   sherpa-onnx-offline.exe nemo_transducer (Parakeet dirs)
  online    sherpa-onnx.exe streaming transducer (zipformer dirs), file fed at once
Latency = the engine's own compute time for the clip, model load excluded (the app keeps
models resident): whisper total - load; sherpa "Elapsed seconds". RTF = latency / audio.
WER: English = ../../bench/wer.py normaliser; Hebrew = strip nikud + punctuation, split.

usage (on the PC, from the bench dir):
  python asr_win.py --bin BIN --models MODELS --audio AUDIO --manifest M.json --lang en|he \
      --run whisper:ggml-small-q5_1.bin:8 --run offline:parakeet-dir:4 ... --out results/x.jsonl
"""
import argparse, json, os, re, subprocess, sys, time, pathlib

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from wer import edit_distance, normalise  # copied next to this file on the PC

BELOW_NORMAL = 0x00004000
NIKUD = re.compile(r"[֑-ׇ]")


def he_words(s):
    s = NIKUD.sub("", s)
    s = re.sub(r"[^\w\s]", " ", s)
    return s.split()


def words(s, lang):
    return he_words(s) if lang == "he" else normalise(s)


def run(cmd):
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, creationflags=BELOW_NORMAL)
    wall = (time.perf_counter() - t0) * 1000
    out, err = p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace")
    if p.returncode != 0:
        raise RuntimeError(err[-1500:])
    return out, err, wall


def whisper(a, model, th, wav, ac, exe="whisper-cpu/Release/whisper-cli.exe"):
    cmd = [str(a.bin / exe), "-m", str(a.models / model), "-f", str(wav),
           "-t", str(th), "-l", a.lang, "-nt", "-bs", "1", "-bo", "1"]
    if ac:
        dur = os.path.getsize(wav) / 32000
        cmd += ["-ac", str(min(1500, int(dur * 50) + 64))]
    out, err, wall = run(cmd)
    tim = {k: float(v) for k, v in re.findall(r"whisper_print_timings:\s+(\w+) time =\s+([\d.]+) ms", err)}
    hyp = " ".join(l.strip() for l in out.splitlines() if l.strip())
    return hyp, tim.get("total", wall) - tim.get("load", 0.0), wall


SHERPA = "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-no-tts/bin"


def offline(a, d, th, wav, _):
    d = a.models / d
    cmd = [str(a.bin / SHERPA / "sherpa-onnx-offline.exe"), f"--encoder={d}/encoder.int8.onnx",
           f"--decoder={d}/decoder.int8.onnx", f"--joiner={d}/joiner.int8.onnx", f"--tokens={d}/tokens.txt",
           "--model-type=nemo_transducer", f"--num-threads={th}", str(wav)]
    return sherpa_parse(*run(cmd))


def online(a, d, th, wav, _):
    d = a.models / d
    cmd = [str(a.bin / SHERPA / "sherpa-onnx.exe"), f"--encoder={d}/encoder.int8.onnx",
           f"--decoder={d}/decoder.int8.onnx", f"--joiner={d}/joiner.int8.onnx", f"--tokens={d}/tokens.txt",
           f"--num-threads={th}", str(wav)]
    return sherpa_parse(*run(cmd))


def sherpa_parse(out, err, wall):
    s = out + err
    m = re.findall(r'"text":\s*"(.*?)"', s)
    hyp = m[-1] if m else ""
    if not m:  # sherpa-onnx.exe prints the text on its own line after the wav path
        lines = [l for l in s.splitlines() if l.strip() and not re.search(r"Elapsed|Real time|num threads|wav|Started|Done|decoding method|\.cc:|Creat|Reading|Wait|Num", l)]
        hyp = lines[-1].strip() if lines else ""
    el = re.search(r"Elapsed seconds:\s*([\d.]+)", s)
    return hyp, (float(el.group(1)) * 1000 if el else wall), wall


def whisper_vk(a, model, th, wav, ac):
    return whisper(a, model, th, wav, ac, exe="whisper-vk2/whisper-cli.exe")


ENG = {"whisper": whisper, "whisper_vk": whisper_vk, "offline": offline, "online": online}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", type=pathlib.Path, required=True)
    ap.add_argument("--models", type=pathlib.Path, required=True)
    ap.add_argument("--audio", type=pathlib.Path, required=True)
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--lang", default="en")
    ap.add_argument("--run", action="append", required=True, help="engine:model:threads[:ac]")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    man = json.load(open(a.manifest, encoding="utf-8"))["utterances"]
    with open(a.out, "a", encoding="utf-8") as fo:
        for spec in a.run:
            eng, model, th, *rest = spec.split(":")
            ac = bool(rest)
            fn = ENG[eng]
            fn(a, model, int(th), a.audio / f"{man[0]['id']}.wav", ac)  # warm-up (file cache), discarded
            errs = n = 0; lat = []; aud = 0.0
            for u in man:
                hyp, ms, wall = fn(a, model, int(th), a.audio / f"{u['id']}.wav", ac)
                r, h = words(u["ref"], a.lang), words(hyp, a.lang)
                e = edit_distance(r, h); errs += e; n += len(r); lat.append(ms); aud += u["dur_s"]
                fo.write(json.dumps({"kind": "clip", "spec": spec, "lang": a.lang, "clip": u["id"], "dur_s": u["dur_s"],
                                     "hyp": hyp, "errors": e, "ref_words": len(r), "compute_ms": ms,
                                     "wall_ms": wall}, ensure_ascii=False) + "\n"); fo.flush()
            lat.sort()
            row = {"kind": "summary", "spec": spec, "lang": a.lang, "clips": len(man), "wer": errs / n,
                   "rtf": sum(lat) / 1000 / aud, "mean_ms": sum(lat) / len(lat), "p90_ms": lat[int(0.9 * len(lat)) - 1],
                   "max_ms": lat[-1]}
            fo.write(json.dumps(row) + "\n"); fo.flush()
            print(json.dumps(row), flush=True)


if __name__ == "__main__":
    main()
