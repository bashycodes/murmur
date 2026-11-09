#!/usr/bin/env python3
"""ASR benchmark: WER + real-time factor for whisper.cpp models (and Parakeet via sherpa-onnx).

For each (model, threads):
  * cold: evict the model file from the page cache (posix_fadvise DONTNEED, no root needed), then run
    whisper-cli on jfk.wav; wall time = what the very first dictation after boot would feel like
    (process start + model read from disk + inference).
  * warm: run every manifest clip with the model file page-cached. We record process wall time and
    whisper's own timings; `infer_ms` = total - load is the latency an app with the model already
    resident in memory would see. RTF = infer_ms / audio_ms.
Decoding is greedy (-bs 1 -bo 1, temperature fallback left on), -l en, flash-attn on (default).

Writes JSON lines to $CACHE/results/asr-<ts>.jsonl (per-session path, never overwritten) and prints a
summary table. Usage:
  python3 -I tools/probes/bench/bench_asr.py [--models tiny,base,small,turbo,parakeet] [--threads 4,8]
"""
import argparse, json, os, pathlib, re, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from wer import wer_counts  # noqa: E402

CACHE = pathlib.Path(os.environ.get("CACHE", pathlib.Path.home() / ".cache/flow-clone"))
WCLI = CACHE / "whisper.cpp/build/bin/whisper-cli"
SHERPA = CACHE / "sherpa-onnx/bin/sherpa-onnx-offline"
PK = CACHE / "models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"
MODELS = {
    "tiny": "ggml-tiny-q5_1.bin",
    "base": "ggml-base-q5_1.bin",
    "small": "ggml-small-q5_1.bin",
    "turbo": "ggml-large-v3-turbo-q5_0.bin",
}


def evict(path: pathlib.Path):
    for p in ([path] if path.is_file() else [q for q in path.glob("*") if q.is_file()]):
        fd = os.open(p, os.O_RDONLY)
        try:
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        finally:
            os.close(fd)


def warm_cache(path: pathlib.Path):
    for p in ([path] if path.is_file() else [q for q in path.glob("*") if q.is_file()]):
        with open(p, "rb") as f:
            while f.read(1 << 24):
                pass


AUDIO_CTX_AUTO = False


def run_whisper(model: pathlib.Path, wav: pathlib.Path, threads: int):
    cmd = [str(WCLI), "-m", str(model), "-f", str(wav), "-t", str(threads), "-l", "en", "-nt",
           "-bs", "1", "-bo", "1"]
    if AUDIO_CTX_AUTO:
        # Encoder normally always processes a 30 s window (1500 frames). Shrink it to the clip length
        # (50 frames/s) plus ~1 s margin; trades some accuracy for latency on short dictations.
        dur = os.path.getsize(wav) / 32000  # 16 kHz PCM16 mono, header negligible
        cmd += ["-ac", str(min(1500, int(dur * 50) + 64))]
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, text=True)
    wall = (time.perf_counter() - t0) * 1000
    if p.returncode != 0:
        raise RuntimeError(p.stderr[-2000:])
    tim = {k: float(v) for k, v in re.findall(r"whisper_print_timings:\s+(\w+) time =\s+([\d.]+) ms", p.stderr)}
    hyp = " ".join(l.strip() for l in p.stdout.splitlines() if l.strip())
    return hyp, wall, tim


def run_parakeet(wav: pathlib.Path, threads: int):
    cmd = [str(SHERPA), f"--encoder={PK}/encoder.int8.onnx", f"--decoder={PK}/decoder.int8.onnx",
           f"--joiner={PK}/joiner.int8.onnx", f"--tokens={PK}/tokens.txt", "--model-type=nemo_transducer",
           f"--num-threads={threads}", str(wav)]
    env = dict(os.environ, LD_LIBRARY_PATH=str(CACHE / "sherpa-onnx/lib"))
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    wall = (time.perf_counter() - t0) * 1000
    if p.returncode != 0:
        raise RuntimeError(p.stderr[-2000:])
    out = p.stdout + p.stderr
    m = re.search(r'"text":\s*"(.*?)"', out)
    hyp = m.group(1) if m else ""
    tim = {}
    el = re.search(r"Elapsed seconds:\s*([\d.]+)", out)
    if el:
        tim["infer"] = float(el.group(1)) * 1000  # decode time reported by sherpa (excl. model load)
    return hyp, wall, tim


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", default="tiny,base,small,turbo,parakeet")
    ap.add_argument("--threads", default="4,8")
    ap.add_argument("--manifest", default="asr_manifest.json")
    ap.add_argument("--audio-ctx-auto", action="store_true", help="whisper -ac sized to each clip")
    a = ap.parse_args()
    global AUDIO_CTX_AUTO
    AUDIO_CTX_AUTO = a.audio_ctx_auto
    man = json.loads((HERE / a.manifest).read_text())["utterances"]
    audio = CACHE / "audio"
    res_dir = CACHE / "results"; res_dir.mkdir(exist_ok=True)
    out = res_dir / f"asr-{pathlib.Path(a.manifest).stem}{'-ac' if a.audio_ctx_auto else ''}-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
    summary = []
    with open(out, "w") as fo:
        for name in a.models.split(","):
            mpath = PK if name == "parakeet" else CACHE / "models" / MODELS[name]
            runner = (lambda w, t: run_parakeet(w, t)) if name == "parakeet" else \
                     (lambda w, t, m=mpath: run_whisper(m, w, t))
            for th in map(int, a.threads.split(",")):
                evict(mpath)
                hyp, cold_wall, tim = runner(audio / "jfk.wav", th)
                rec = {"kind": "cold", "model": name, "threads": th, "clip": "jfk", "wall_ms": cold_wall,
                       "timings": tim, "load1": os.getloadavg()[0]}
                fo.write(json.dumps(rec) + "\n")
                warm_cache(mpath)
                runner(audio / "jfk.wav", th)  # discard one warm-up run
                errs = words = 0; infer = audio_ms = wall_sum = 0.0
                for u in man:
                    hyp, wall, tim = runner(audio / f"{u['id']}.wav", th)
                    e, n = wer_counts(u["ref"], hyp)
                    errs += e; words += n
                    inf = tim.get("infer", tim.get("total", wall) - tim.get("load", 0.0))
                    infer += inf; audio_ms += u["dur_s"] * 1000; wall_sum += wall
                    fo.write(json.dumps({"kind": "warm", "model": name, "threads": th, "clip": u["id"],
                                         "dur_s": u["dur_s"], "hyp": hyp, "errors": e, "ref_words": n,
                                         "wall_ms": wall, "infer_ms": inf, "timings": tim,
                                         "load1": os.getloadavg()[0]}) + "\n")
                    fo.flush()
                row = dict(model=name + ("+ac" if a.audio_ctx_auto and name != "parakeet" else ""), threads=th, wer=errs / words, rtf_infer=infer / audio_ms,
                           rtf_wall=wall_sum / audio_ms, cold_jfk_ms=cold_wall,
                           mean_infer_ms=infer / len(man), load1=os.getloadavg()[0])
                summary.append(row)
                print(json.dumps(row), flush=True)
        fo.write(json.dumps({"kind": "summary", "rows": summary}) + "\n")
    print("\n| model | thr | WER | RTF (infer) | RTF (process wall) | cold jfk wall ms | mean infer ms/clip |")
    print("|---|---|---|---|---|---|---|")
    for r in summary:
        print(f"| {r['model']} | {r['threads']} | {r['wer']*100:.1f}% | {r['rtf_infer']:.3f} | {r['rtf_wall']:.3f} | "
              f"{r['cold_jfk_ms']:.0f} | {r['mean_infer_ms']:.0f} |")
    print("results:", out)


if __name__ == "__main__":
    main()
