// Command line of run.sh (see docs/testing/windows-e2e.md).
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** Gitignored per-machine settings (KEY=value lines), next to run.sh. */
export const LOCAL_ENV = join(dirname(fileURLToPath(import.meta.url)), "..", "local.env");

export const SCENARIOS = ["a", "b", "c", "d", "e", "f", "g"];
export const COMMANDS = ["run", "restore", "status", "vbcable-install", "clips", "selftest"];
/** Scenario d's dictation profiles: quiet = the seed; owner = the owner's sounds, ducking and mic. */
export const PROFILES = ["quiet", "owner"];
export const KEYS = ["RAlt", "LAlt"];

export const USAGE = `usage: tools/e2e-windows/run.sh [run] (--installer NAME | --installer-local PATH) [options]
       tools/e2e-windows/run.sh restore | status | vbcable-install | clips | selftest

  --installer NAME       installer in the laptop's Downloads (or a full Windows path)
  --installer-local PATH local installer (e.g. a CI artifact): copied to the laptop, then used
  --allow-downgrade      allow an installer older than the owner's installed version
  --scenarios a,b,...    subset of a-g (default: all)
  --no-restore           leave the test install's data in place (restore later with: run.sh restore)
  --no-wait              fail at once if another run holds the laptop (default: wait in line)
  --host USER@HOST       ssh destination (default: $MURMUR_E2E_HOST, then
                         MURMUR_E2E_HOST= in tools/e2e-windows/local.env)
  --out DIR              results root (default tools/e2e-windows/results)
 scenario d (double-tap), every combination of:
  --keys K,K             keys to double-tap: RAlt,LAlt (default both)
  --tap-ms MS,MS         how long each tap is held (default 80,120,160)
  --gaps MS,MS           release-to-press gap (default 120,200,300,400)
  --profiles P,P         quiet (sounds/ducking off, VB-Cable mic) and/or owner
                         (the owner's sounds, ducking and mic, all muted) (default both)
  --tries N              tries per combination (default 4)
  --jitter MS            random +-MS on every tap and gap (default 20, 0 = exact)
  --altgr                also try Right Alt as AltGr sends it (fake Left Ctrl + Right Alt)
  --layout he            switch the target window to the Hebrew layout first (AltGr for real)`;

/** KEY=value lines of a local.env file ({} if there is none). */
export function readEnvFile(path) {
  if (!existsSync(path)) return {};
  const m = {};
  for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
    const x = /^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$/.exec(line);
    if (x) m[x[1]] = x[2].replace(/^["']|["']$/g, "");
  }
  return m;
}

/** The laptop's ssh destination: $MURMUR_E2E_HOST, then local.env. No built-in default. */
export function defaultHost(env = process.env, envFile = LOCAL_ENV) {
  return env.MURMUR_E2E_HOST || readEnvFile(envFile).MURMUR_E2E_HOST || null;
}

/** Commands that never reach the laptop. */
const OFFLINE = ["help", "clips"];

const list = (v, map = (x) => x) => v.split(",").map((s) => s.trim()).filter(Boolean).map(map);
const numbers = (flag, v) => {
  const xs = list(v, Number);
  if (!xs.length || xs.some((x) => !Number.isFinite(x) || x < 0)) throw new Error(`${flag}: expected numbers, got ${v}`);
  return xs;
};
const oneOf = (flag, v, allowed) => {
  const xs = list(v);
  for (const x of xs) if (!allowed.includes(x)) throw new Error(`${flag}: unknown ${x} (one of ${allowed.join(", ")})`);
  return xs;
};

export function parseArgs(argv) {
  const a = {
    command: "run",
    installer: null,
    installerLocal: null,
    scenarios: [...SCENARIOS],
    restore: true,
    wait: true,
    host: defaultHost(),
    out: null,
    keys: [...KEYS],
    taps: [80, 120, 160],
    gaps: [120, 200, 300, 400],
    profiles: [...PROFILES],
    tries: 4,
    jitter: 20,
    altgr: false,
    layout: null,
    allowDowngrade: false,
  };
  const rest = [...argv];
  if (rest[0] && !rest[0].startsWith("-")) {
    if (!COMMANDS.includes(rest[0])) throw new Error(`unknown command: ${rest[0]}\n${USAGE}`);
    a.command = rest.shift();
  }
  const val = (flag) => {
    const v = rest.shift();
    if (v === undefined) throw new Error(`${flag} needs a value`);
    return v;
  };
  while (rest.length) {
    const f = rest.shift();
    switch (f) {
      case "--installer": a.installer = val(f); break;
      case "--installer-local": a.installerLocal = val(f); break;
      case "--scenarios": a.scenarios = oneOf(f, val(f).toLowerCase(), SCENARIOS); break;
      case "--tries": a.tries = numbers(f, val(f))[0]; break;
      case "--gaps": a.gaps = numbers(f, val(f)); break;
      case "--tap-ms": a.taps = numbers(f, val(f)); break;
      case "--keys": a.keys = oneOf(f, val(f), KEYS); break;
      case "--profiles": a.profiles = oneOf(f, val(f), PROFILES); break;
      case "--jitter": a.jitter = numbers(f, val(f))[0]; break;
      case "--altgr": a.altgr = true; break;
      case "--allow-downgrade": a.allowDowngrade = true; break;
      case "--layout": a.layout = oneOf(f, val(f), ["he"])[0]; break;
      case "--no-restore": a.restore = false; break;
      case "--no-wait": a.wait = false; break;
      case "--host": a.host = val(f); break;
      case "--out": a.out = val(f); break;
      case "-h": case "--help": a.command = "help"; break;
      default: throw new Error(`unknown option: ${f}\n${USAGE}`);
    }
  }
  if (a.command === "run" && !a.installer === !a.installerLocal) throw new Error(`give one of --installer or --installer-local\n${USAGE}`);
  if (a.installerLocal && !existsSync(a.installerLocal)) throw new Error(`no such file: ${a.installerLocal}`);
  if (!a.host && !OFFLINE.includes(a.command)) {
    throw new Error("no laptop host: set MURMUR_E2E_HOST=user@host, put MURMUR_E2E_HOST=user@host in tools/e2e-windows/local.env (gitignored), or pass --host");
  }
  return a;
}
