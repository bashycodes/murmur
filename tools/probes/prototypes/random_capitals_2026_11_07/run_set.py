#!/usr/bin/env python3
"""Run every WAV of a directory through `murmur-cli transcribe --json`.

Probe for docs/findings/2026-11-07-random-capitals.md. Writes one row per
clip, `{"clip", "raw" (asr_text, after joins), "rules" (final_text),
"status"}`, to OUT.jsonl (keep it outside the repo: owner audio gives
owner text). Extra arguments go to murmur-cli (model, session, ...).
Env `MURMUR_SEGMENT_JOIN` passes through (bench knob).

Usage: run_set.py MURMUR_CLI WAV_DIR OUT.jsonl -- [murmur-cli transcribe args]
"""
import json
import subprocess
import sys
from pathlib import Path


def main():
    cli, wavs, out = sys.argv[1], Path(sys.argv[2]), sys.argv[3]
    extra = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    with open(out, "w", encoding="utf-8") as f:
        for wav in sorted(wavs.glob("*.wav")):
            p = subprocess.run(
                [cli, "transcribe", *extra, "--json", str(wav)],
                capture_output=True,
                text=True,
            )
            if p.returncode != 0:
                print(f"{wav.stem}: exit {p.returncode}", file=sys.stderr)
                continue
            r = json.loads(p.stdout)
            row = {
                "clip": wav.stem,
                "raw": r.get("asr_text"),
                "rules": r.get("final_text"),
                "status": r.get("status"),
            }
            f.write(json.dumps(row) + "\n")
            f.flush()
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
