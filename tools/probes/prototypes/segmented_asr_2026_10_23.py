#!/usr/bin/env python3
"""Dated probe (2026-10-23) for docs/proposals/streaming-asr-latency.md.

Questions it answers on this box (i7-6700, release flow-cli):
  A. ASR time vs clip length (whole clip), Parakeet and whisper base q5_1:
     is it linear, and where are the knees (whisper 29 s audio_ctx
     boundary, 30 s window, Parakeet attention)?
  B. Pause-segmented transcription: split a long clip at pauses >= P ms,
     transcribe each piece alone, join; WER vs the whole-clip WER, and the
     "tail" ASR time (= what would remain after release).
  C. The commit wait behind an in-flight live partial (total_ms - asr_ms)
     with partial_interval_ms = 1500 and real-time pushing (--realtime).

Not promoted to a permanent tool; the consumer is the proposal doc's
tables (copy the printed summary).  Inputs: LibriSpeech clips + refs from
tools/probes/bench/asr_manifest_extended.json, in $AUDIO_DIR.  Outputs
(WAVs, JSONL) go to $OUT_DIR (default ~/Claude/research/wispr/.cache/
segmented-2026-10-23/<timestamp>/) - per-run directory, never overwritten.

Usage:
  python3 -I tools/probes/prototypes/segmented_asr_2026_10_23.py \
      --cli target/release/flow-cli --whisper ~/…/ggml-base-q5_1.bin \
      --parakeet ~/…/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 \
      [--parts A,B,C] [--threads 4]
"""
import argparse, json, os, subprocess, sys, time, wave
from array import array

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "bench"))
from wer import wer_counts  # noqa: E402

SR = 16000


def read_wav(p):
    with wave.open(p) as w:
        assert w.getframerate() == SR and w.getnchannels() == 1 and w.getsampwidth() == 2
        a = array("h")
        a.frombytes(w.readframes(w.getnframes()))
        return a


def write_wav(p, a):
    with wave.open(p, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(a.tobytes())


def build_clips(manifest, audio_dir, out, targets, gap_ms):
    """Concatenate consecutive utterances with `gap_ms` of silence until each
    target length is reached. Returns [(path, dur_s, ref)]."""
    utts = [u for u in manifest["utterances"]
            if os.path.exists(os.path.join(audio_dir, u["id"] + ".wav"))]
    gap = array("h", [0] * (SR * gap_ms // 1000))
    clips, i = [], 0
    for t in targets:
        pcm, refs = array("h"), []
        while len(pcm) < t * SR:
            u = utts[i % len(utts)]
            i += 1
            if pcm:
                pcm.extend(gap)
            pcm.extend(read_wav(os.path.join(audio_dir, u["id"] + ".wav")))
            refs.append(u["ref"])
        p = os.path.join(out, f"long_{t:03d}s.wav")
        write_wav(p, pcm)
        clips.append((p, len(pcm) / SR, " ".join(refs)))
    return clips


def pauses(pcm, min_pause_ms, frame_ms=20):
    """Cut points (sample index, middle of each pause >= min_pause_ms).
    Energy only, noise floor = 10th percentile frame RMS (roughly what the
    core's streaming D16 VAD sees)."""
    n = SR * frame_ms // 1000
    rms = []
    for k in range(0, len(pcm) - n + 1, n):
        f = pcm[k:k + n]
        rms.append((sum(x * x for x in f) / n) ** 0.5)
    floor = sorted(rms)[len(rms) // 10]
    thr = max(floor * 3.0, 150.0)
    cuts, run = [], 0
    for j, r in enumerate(rms):
        if r < thr:
            run += 1
        else:
            if run * frame_ms >= min_pause_ms:
                cuts.append((j - run // 2) * n)
            run = 0
    return cuts


# flow-cli pushes the whole file at once; the default 30 s ring would drop
# everything past ~30 s (dropped_samples) - a CLI artefact, not a shell one.
BIG_RING = {"ring_capacity_samples": SR * 130}


def transcribe(cli, engine_args, wav, threads, session=None, realtime=False):
    if session is None and not realtime:
        session = BIG_RING
    cmd = [cli, "transcribe", *engine_args, "--threads", str(threads), "--json"]
    if session is not None:
        sp = wav + ".session.json"
        with open(sp, "w") as f:
            json.dump(session, f)
        cmd += ["--session", sp]
    if realtime:
        cmd.append("--realtime")
    cmd.append(wav)
    t0 = time.time()
    p = subprocess.run(cmd, capture_output=True, text=True, env={**os.environ, "RUST_LOG": "off"})
    if p.returncode != 0:
        raise RuntimeError(p.stderr[-2000:])
    r = json.loads(p.stdout)
    r["_wall_s"] = time.time() - t0
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--whisper")
    ap.add_argument("--parakeet")
    ap.add_argument("--audio", default=os.path.expanduser("~/Claude/research/wispr/.cache/audio"))
    ap.add_argument("--manifest", default=os.path.join(HERE, "..", "bench", "asr_manifest_extended.json"))
    ap.add_argument("--out", default=None)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--parts", default="A,B,C")
    ap.add_argument("--targets", default="5,10,15,20,25,28,32,40,60")
    ap.add_argument("--gap-ms", type=int, default=700)
    ap.add_argument("--pause-ms", default="500,800")
    args = ap.parse_args()
    out = args.out or os.path.expanduser(
        f"~/Claude/research/wispr/.cache/segmented-2026-10-23/{time.strftime('%Y%m%d-%H%M%S')}")
    os.makedirs(out, exist_ok=True)
    manifest = json.load(open(args.manifest))
    targets = [int(x) for x in args.targets.split(",")]
    clips = sorted(build_clips(manifest, args.audio, out, targets, args.gap_ms), key=lambda c: c[1])
    engines = []
    if args.parakeet:
        engines.append(("parakeet", ["--engine", "parakeet", "--model", args.parakeet]))
    if args.whisper:
        engines.append(("whisper-base-q5_1", ["--model", args.whisper]))
    parts = set(args.parts.split(","))
    log = open(os.path.join(out, "rows.jsonl"), "w")

    def emit(row):
        log.write(json.dumps(row) + "\n")
        log.flush()

    whole = {}
    if "A" in parts or "B" in parts:
        print("== A: whole clip ==")
        for name, ea in engines:
            for path, dur, ref in clips:
                r = transcribe(args.cli, ea, path, args.threads)
                t = r["timings"]
                e, n = wer_counts(ref, r["asr_text"])
                whole[(name, path)] = (e, n)
                emit({"part": "A", "engine": name, "dur_s": dur, "asr_ms": t["asr_ms"],
                      "total_ms": t["total_ms"], "asr_audio_ms": t.get("asr_audio_ms"),
                      "dropped": t.get("dropped_samples"),
                      "wer": e / n, "hyp": r["asr_text"], "ref": ref})
                print(f"{name:18s} {dur:5.1f}s asr {t['asr_ms']:6d} ms  RTF {t['asr_ms']/1000/dur:.3f}  "
                      f"total-asr {t['total_ms']-t['asr_ms']:4d}  WER {100*e/n:5.2f}%")
    if "B" in parts:
        print("== B: pause-segmented ==")
        for pause_ms in [int(x) for x in args.pause_ms.split(",")]:
            for name, ea in engines:
                agg = [0, 0, 0, 0]
                for path, dur, ref in clips:
                    if dur < 15:
                        continue
                    pcm = read_wav(path)
                    cuts = pauses(pcm, pause_ms)
                    bounds = [0, *cuts, len(pcm)]
                    texts, times = [], []
                    for k in range(len(bounds) - 1):
                        seg = pcm[bounds[k]:bounds[k + 1]]
                        sp = f"{path}.p{pause_ms}.s{k}.wav"
                        write_wav(sp, seg)
                        r = transcribe(args.cli, ea, sp, args.threads)
                        texts.append(r["asr_text"])
                        times.append(r["timings"]["asr_ms"])
                    hyp = " ".join(t for t in texts if t)
                    e, n = wer_counts(ref, hyp)
                    we, wn = whole[(name, path)]
                    agg[0] += e; agg[1] += n; agg[2] += we; agg[3] += wn
                    emit({"part": "B", "engine": name, "pause_ms": pause_ms, "dur_s": dur,
                          "segments": len(texts), "seg_ms": times,
                          "seg_s": [(bounds[k + 1] - bounds[k]) / SR for k in range(len(bounds) - 1)],
                          "wer": e / n, "wer_whole": we / wn, "hyp": hyp, "ref": ref})
                    print(f"{name:18s} P={pause_ms} {dur:5.1f}s segs {len(texts):2d} "
                          f"max seg {max(bounds[k+1]-bounds[k] for k in range(len(bounds)-1))/SR:4.1f}s "
                          f"tail asr {times[-1]:5d} ms (sum {sum(times):6d})  "
                          f"WER seg {100*e/n:5.2f}% whole {100*we/wn:5.2f}%")
                print(f"  {name} P={pause_ms}: pooled WER seg {100*agg[0]/agg[1]:.2f}% vs whole {100*agg[2]/agg[3]:.2f}%")
    if "C" in parts:
        print("== C: commit wait behind a live partial (realtime push, partial 1500 ms) ==")
        for name, ea in engines:
            for path, dur, ref in clips:
                if dur < 20 or dur > 33:
                    continue
                for sess, label in [({"partial_interval_ms": 1500}, "partials"), ({}, "no-partials")]:
                    r = transcribe(args.cli, ea, path, args.threads, session=sess, realtime=True)
                    t = r["timings"]
                    emit({"part": "C", "engine": name, "dur_s": dur, "mode": label,
                          "asr_ms": t["asr_ms"], "total_ms": t["total_ms"]})
                    print(f"{name:18s} {dur:5.1f}s {label:11s} asr {t['asr_ms']:6d} total {t['total_ms']:6d} "
                          f"gap {t['total_ms']-t['asr_ms']:5d} ms")
    print("rows:", os.path.join(out, "rows.jsonl"))


if __name__ == "__main__":
    main()
