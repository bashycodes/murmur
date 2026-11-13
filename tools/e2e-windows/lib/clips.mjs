// Generate the synthetic TTS clips (clips/clips.json) with espeak-ng.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { durationMs, joinWavs, readWav, writeWav } from "./wav.mjs";

/** Path of an espeak-ng binary (system one, else the user-space unpack). */
export function espeakPath(clipsDir) {
  const r = spawnSync(join(clipsDir, "fetch-espeak.sh"), [], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`fetch-espeak.sh failed: ${r.stderr}`);
  return r.stdout.trim().split("\n").pop();
}

/**
 * Write <outDir>/<id>.wav for every clip; returns {id: {path, lang, expect, ms}}.
 * Clips already there are reused (same text = same file).
 */
export function generateClips(clipsDir, outDir) {
  const spec = JSON.parse(readFileSync(join(clipsDir, "clips.json"), "utf8")).clips;
  mkdirSync(outDir, { recursive: true });
  const espeak = espeakPath(clipsDir);
  const out = {};
  for (const [id, c] of Object.entries(spec)) {
    const path = join(outDir, `${id}.wav`);
    if (!existsSync(path)) {
      const parts = c.parts.map((p, i) => {
        const tmp = join(outDir, `${id}.part${i}.wav`);
        // -s 150: a little slower than the default, clearer for ASR.
        const r = spawnSync(espeak, ["-v", p.voice, "-s", "150", "-w", tmp, p.text], { encoding: "utf8" });
        if (r.status !== 0) throw new Error(`espeak-ng ${id}: ${r.stderr}`);
        return readWav(readFileSync(tmp));
      });
      writeFileSync(path, writeWav(joinWavs(parts, 300)));
    }
    out[id] = { path, lang: c.lang, expect: c.expect, ms: durationMs(readWav(readFileSync(path))) };
  }
  return out;
}
