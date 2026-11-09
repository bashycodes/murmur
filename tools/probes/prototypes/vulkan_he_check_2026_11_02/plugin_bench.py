#!/usr/bin/env python3
"""Hebrew STT latency through the installed murmur_whisper_vulkan.dll (dated probe, 2026-11-02;
runs ON the laptop). Drives the plugin's C ABI (crates/murmur-whisper/src/lib.rs, ABI v1) with
ctypes, the same calls the app makes (core_adapter::whisper_engine: fit_ctx false, the app's
default thread count), so the decode is the app's own. One resident engine, clips one after
another, so latency = murmur_whisper_transcribe wall time (no model load, no process start).

  python plugin_bench.py --dll "<install>/murmur_whisper_vulkan.dll" --model M.bin \
      --audio audio-he --manifest he_manifest.json [--ids a,b,...] [--limit N] --out x.jsonl
  MURMUR_WHISPER_DEVICE=cpu forces the plugin's CPU path (set it in the environment).

Some FLEURS he recordings sit under the app's silence gate (RMS < 0.003, murmur-core
whisper.rs SILENCE_RMS), where the engine returns "" without decoding. Those clips are
scaled to RMS 0.05 unless --no-lift (the row records rms + gain).

WER: strip nikud + punctuation, split (as laptop_bench_2026_10_30/asr_win.py).
"""
import argparse, array, ctypes, json, os, pathlib, re, statistics, sys, time, wave

NIKUD = re.compile(r"[֑-ׇ]")
GATE_RMS = 0.003
LIFT_RMS = 0.05


def he_words(s):
    s = NIKUD.sub("", s)
    s = re.sub(r"[^\w\s]", " ", s)
    return s.split()


def edit_distance(a, b):
    d = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        prev, d[0] = d[0], i
        for j, y in enumerate(b, 1):
            prev, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, prev + (x != y))
    return d[len(b)]


def pcm(path, lift):
    with wave.open(str(path)) as w:
        assert w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2, path
        a = array.array("h", w.readframes(w.getnframes()))
    f = [x / 32768.0 for x in a]
    rms = (sum(x * x for x in f) / len(f)) ** 0.5
    gain = LIFT_RMS / rms if lift and 0 < rms < GATE_RMS else 1.0
    return (ctypes.c_float * len(f))(*(x * gain for x in f)), len(f), rms, gain


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dll", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--audio", type=pathlib.Path, required=True)
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--ids", default="")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--threads", type=int, default=max(1, min(8, (os.cpu_count() or 8) // 2)))
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--no-lift", action="store_true")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    os.add_dll_directory(str(pathlib.Path(a.dll).parent))
    L = ctypes.CDLL(a.dll)
    L.murmur_whisper_abi_version.restype = ctypes.c_uint32
    L.murmur_whisper_backends.restype = ctypes.c_void_p
    L.murmur_whisper_load.restype = ctypes.c_void_p
    L.murmur_whisper_load.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    L.murmur_whisper_info.restype = ctypes.c_void_p
    L.murmur_whisper_info.argtypes = [ctypes.c_void_p]
    L.murmur_whisper_transcribe.restype = ctypes.c_int32
    L.murmur_whisper_transcribe.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
                                          ctypes.c_char_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    L.murmur_whisper_free_string.argtypes = [ctypes.c_void_p]

    def take(p):
        s = ctypes.string_at(p).decode("utf-8", "replace")
        L.murmur_whisper_free_string(p)
        return s

    out = open(a.out, "w", encoding="utf-8")

    def emit(row):
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
        out.flush()
        print(json.dumps({k: v for k, v in row.items() if k != "hyp"})[:300], flush=True)

    emit({"kind": "env", "abi": L.murmur_whisper_abi_version(), "backends": json.loads(take(L.murmur_whisper_backends())),
          "MURMUR_WHISPER_DEVICE": os.environ.get("MURMUR_WHISPER_DEVICE"), "threads": a.threads})
    err = ctypes.c_void_p()
    opts = json.dumps({"threads": a.threads, "fit_ctx": False, "vad_model": None}).encode()
    h = L.murmur_whisper_load(a.model.encode(), opts, ctypes.byref(err))
    if not h:
        emit({"kind": "load_error", "error": take(err.value)})
        sys.exit(1)
    info = json.loads(take(L.murmur_whisper_info(h)))
    emit({"kind": "info", **info})

    m = json.load(open(a.manifest, encoding="utf-8"))["utterances"]
    if a.ids:
        want = a.ids.split(",")
        m = [u for u in m if u["id"] in want]
    if a.limit:
        m = m[: a.limit]
    req = json.dumps({"initial_prompt": "", "language": "he", "prior_text": None}).encode()

    def transcribe(u):
        buf, n, rms, gain = pcm(a.audio / f"{u['id']}.wav", not a.no_lift)
        o = ctypes.c_void_p()
        t0 = time.perf_counter()
        rc = L.murmur_whisper_transcribe(h, buf, n, req, None, ctypes.byref(o))
        ms = (time.perf_counter() - t0) * 1000
        s = take(o.value)
        if rc != 0:
            raise RuntimeError(s)
        return json.loads(s)["text"], ms, n / 16000, rms, gain

    # Warm-up: the first decode builds the GPU pipelines (the app pays this once too).
    for u in m[: a.warmup]:
        hyp, ms, *_ = transcribe(u)
        emit({"kind": "warmup", "id": u["id"], "ms": round(ms), "chars": len(hyp)})

    lat, errs, refw = [], 0, 0
    for u in m:
        hyp, ms, dur, rms, gain = transcribe(u)
        r, hw = he_words(u["ref"]), he_words(hyp)
        e = edit_distance(r, hw)
        errs, refw = errs + e, refw + len(r)
        lat.append(ms)
        emit({"kind": "clip", "id": u["id"], "dur_s": round(dur, 2), "ms": round(ms), "rms": round(rms, 5),
              "gain": round(gain, 1), "errs": e, "ref_words": len(r), "hyp": hyp})
    if not lat:
        return
    q = statistics.quantiles(lat, n=10) if len(lat) > 1 else lat * 9
    emit({"kind": "summary", "backend": info["backend"], "device": info["device"], "n": len(lat),
          "p50_ms": round(statistics.median(lat)), "p90_ms": round(q[8]), "max_ms": round(max(lat)),
          "mean_ms": round(statistics.mean(lat)), "wer": round(100 * errs / max(1, refw), 1)})


if __name__ == "__main__":
    main()
