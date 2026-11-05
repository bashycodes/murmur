#!/usr/bin/env python3
"""Time onnxruntime session creation of a sherpa model dir per graph-optimisation level.

Probe for docs/findings/2026-11-05-phone-model-load-time.md (no permanent tool).
Usage: python3 -P ort_session_levels.py MODEL_DIR [--runs N]
Prints, per level and file, the median create time; then the time to load a
pre-optimised copy (session.optimized_model_filepath, written to a temp dir)
with optimisation off, i.e. what an "optimise once, cache" scheme would cost.
"""
import glob, os, statistics, sys, tempfile, time

import onnxruntime as ort

LEVELS = {
    "disable": ort.GraphOptimizationLevel.ORT_DISABLE_ALL,
    "basic": ort.GraphOptimizationLevel.ORT_ENABLE_BASIC,
    "extended": ort.GraphOptimizationLevel.ORT_ENABLE_EXTENDED,
    "all": ort.GraphOptimizationLevel.ORT_ENABLE_ALL,
}


def create(path, level, out=None):
    o = ort.SessionOptions()
    o.graph_optimization_level = level
    o.intra_op_num_threads = 4
    o.inter_op_num_threads = 1
    if out:
        o.optimized_model_filepath = out
    t = time.perf_counter()
    ort.InferenceSession(path, o, providers=["CPUExecutionProvider"])
    return (time.perf_counter() - t) * 1000


def main():
    d = sys.argv[1]
    runs = int(sys.argv[sys.argv.index("--runs") + 1]) if "--runs" in sys.argv else 3
    files = [f for p in ("encoder", "decoder", "joiner")
             for f in sorted(glob.glob(os.path.join(d, p + "*.int8.onnx")))[:1]]
    print("onnxruntime", ort.__version__)
    for name, level in LEVELS.items():
        row = []
        for f in files:
            ms = statistics.median(create(f, level) for _ in range(runs))
            row.append(f"{os.path.basename(f)} {ms:.0f}")
        print(f"{name:9s}", " | ".join(row))
    with tempfile.TemporaryDirectory() as tmp:
        row = []
        for f in files:
            out = os.path.join(tmp, os.path.basename(f))
            create(f, LEVELS["all"], out)
            ms = statistics.median(create(out, LEVELS["disable"]) for _ in range(runs))
            row.append(f"{os.path.basename(f)} {ms:.0f}")
        print("cached   ", " | ".join(row))


if __name__ == "__main__":
    main()
