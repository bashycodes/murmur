// Local cache (clips, espeak-ng, VB-Cable pack, ssh control socket).
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export function cacheDir() {
  const d = process.env.MURMUR_E2E_CACHE ?? join(process.env.XDG_CACHE_HOME ?? join(homedir(), ".cache"), "murmur-e2e");
  mkdirSync(d, { recursive: true });
  return d;
}

/** Official VB-Cable driver pack (vb-audio.com). */
export const VBCABLE_URL = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip";

/** Download and unzip the VB-Cable pack into `dir`; returns the unpacked folder. */
export function fetchVbCable(dir) {
  const zip = join(dir, "dl", "pack.zip");
  const out = join(dir, "x");
  if (!existsSync(join(out, "VBCABLE_Setup_x64.exe"))) {
    mkdirSync(join(dir, "dl"), { recursive: true });
    mkdirSync(out, { recursive: true });
    let r = spawnSync("curl", ["-fsSL", "-o", zip, VBCABLE_URL], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(`download ${VBCABLE_URL} failed: ${r.stderr}`);
    r = spawnSync("unzip", ["-oq", zip, "-d", out], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(`unzip failed: ${r.stderr}`);
  }
  return out;
}
