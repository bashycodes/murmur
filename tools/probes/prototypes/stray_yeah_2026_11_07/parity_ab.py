"""Probe (stray "yeah", 2026-11-07, D72 parity check): run every WAV in a
folder through `flow-cli transcribe --json --level none` and append one row
per clip to OUT.jsonl ({clip, raw, status, segments}). Lines on stderr that
start with a tag (default `[DEBUG-sy72]`, a temporary piece-gain trace in
an instrumented build) go to OUT.debug, prefixed with the clip.

Usage: python3 -I parity_ab.py CLI SESSION.json CLIPS_DIR OUT.jsonl [flow-cli args...]
Evidence (clips, rows) stays outside the repo; prints counts only.
"""

import glob
import json
import os
import subprocess
import sys


def main() -> None:
    cli, session, clips, out, extra = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5:]
    tag = os.environ.get("TAG", "[DEBUG-sy72]")
    env = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(cli))
    done = set()
    if os.path.exists(out):
        done = {json.loads(l)["clip"] for l in open(out) if l.strip()}
    n = 0
    with open(out, "a") as f, open(out[: -len(".jsonl")] + ".debug", "a") as dbg:
        for wav in sorted(glob.glob(os.path.join(clips, "*.wav"))):
            clip = os.path.splitext(os.path.basename(wav))[0]
            if clip in done:
                continue
            argv = [cli, "transcribe", "--json", "--level", "none", "--session", session, *extra, wav]
            p = subprocess.run(argv, capture_output=True, text=True, env=env)
            if p.returncode != 0:
                row = {"clip": clip, "error": p.stderr.strip()[-300:]}
            else:
                r = json.loads(p.stdout)
                row = {"clip": clip, "raw": r["asr_text"], "status": r["status"],
                       "segments": r["timings"].get("segments", 0)}
            for line in p.stderr.splitlines():
                if line.startswith(tag):
                    dbg.write(f"{clip} {line}\n")
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
            f.flush()
            n += 1
    print(f"{out}: {n} new rows")


if __name__ == "__main__":
    main()
