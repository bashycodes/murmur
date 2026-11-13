// Windows e2e harness for Murmur: drives the real app on the owner's
// laptop over SSH (see docs/testing/windows-e2e.md). Entry: run.sh.
import { appendFileSync, copyFileSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Agent } from "./lib/agent.mjs";
import { App } from "./lib/app.mjs";
import { USAGE, parseArgs } from "./lib/args.mjs";
import { generateClips } from "./lib/clips.mjs";
import { LockBusy, RunLock } from "./lib/lock.mjs";
import { cacheDir, fetchVbCable } from "./lib/paths.mjs";
import { AGENT_PORT, CDP_PORT } from "./lib/ports.mjs";
import { Remote } from "./lib/remote.mjs";
import { writeSummary } from "./lib/report.mjs";
import { signalAction } from "./lib/stepmark.mjs";
import { EXIT, compareVersions, computePass, exitCode, installerVersion } from "./lib/verdict.mjs";
import { SCENARIOS } from "./scenarios/index.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REMOTE_DIR = "murmur-e2e";
const CABLE_IN = "CABLE Input"; // waveOut name the clips are played into
export const CABLE_MIC = "CABLE Output (VB-Audio Virtual Cable)"; // what the app records from
const AUDIO_DEFAULTS = "audio-defaults.json"; // in the work dir; deleted once restored

function stamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

/** The prefs merged over the owner's for the dictation scenarios (c-f). */
export const SEED_PATCH = {
  microphone: CABLE_MIC,
  onboarded: true,
  // Quiet and side-effect free on the owner's laptop.
  enable_sounds: false,
  mute_audio_while_dictating: false,
  launch_at_login: false,
  // Language by detection, not by the keyboard layout (scenario f).
  keyboard_language: false,
  languages: [],
  developer: { debug_log: true },
};

/**
 * SIGINT / SIGTERM / SIGHUP: the run in progress registers its clean-up here.
 * The first signal runs it (restore audio, restore data, close pads,
 * relaunch the owner's app), then exits 130; the laptop step in progress
 * finishes first (lib/ssh-shield.sh). A second one within 3 s forces: the
 * shield stops the ssh in flight, the laptop step is stopped through its
 * marker (onForce), then the clean-up runs. Later signals are ignored.
 */
let onSignal = null;
let onForce = null;
let signalled = false;
let lastSignalAt = null;
let forced = false;
/**
 * Let a pending signal reach its handler. ssh runs under the shield, so a
 * Ctrl+C during a spawnSync(ssh) does not stop it, but node's handler only
 * runs once the event loop turns: without this, an error path (a forced
 * stop, a timeout) could finish and exit before the handler saw the signal.
 */
const settle = () => new Promise((r) => setTimeout(r, 50));
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(sig, () => {
    const now = Date.now();
    const action = signalAction(lastSignalAt, now, forced);
    lastSignalAt = now;
    if (action === "ignore") return console.error(`${sig}: clean-up already running, please wait`);
    if (action === "force") {
      forced = true;
      console.error(`${sig} again within 3 s: forcing: stopping the remote step in progress, then cleaning up`);
      try {
        onForce?.();
      } catch (e) {
        console.error(`stopping the remote step: ${e.message}`);
      }
      return;
    }
    signalled = true;
    console.error(`${sig}: cleaning up (restoring the owner's data and audio) before exiting; Ctrl+C again within 3 s to stop the laptop step in progress first`);
    Promise.resolve(onSignal ? onSignal(sig) : null).finally(() => process.exit(EXIT.interrupted));
  });
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (e) {
    console.error(e.message);
    return EXIT.usage;
  }
  if (args.command === "help") {
    console.log(USAGE);
    return EXIT.ok;
  }

  const runId = stamp();
  const outRoot = args.out ?? join(HERE, "results");
  const stem = basename(args.installerLocal ?? args.installer ?? "", ".exe").replace(/^.*\\/, "");
  const dir = args.command === "run" ? join(outRoot, `${runId}-${stem}`) : null;
  if (dir) mkdirSync(join(dir, "shots"), { recursive: true });
  const log = (s) => {
    const line = `${new Date().toISOString().slice(11, 23)} ${s}`;
    console.log(line);
    if (dir) appendFileSync(join(dir, "run.log"), line + "\n");
  };
  if (args.command === "clips") {
    console.log(JSON.stringify(generateClips(join(HERE, "clips"), join(cacheDir(), "clips")), null, 1));
    return EXIT.ok;
  }
  // Laptop steps an earlier, killed run left running and that had to be
  // stopped (lib/stepmark.mjs). run() records them as clean-up errors.
  const orphans = [];
  const remote = new Remote({ host: args.host, dir: REMOTE_DIR, log, onOrphan: (m) => orphans.push(m) });
  remote.sync(join(HERE, "ps"));
  if (args.command === "status") {
    console.log(remote.ps("appdata.ps1", ["-Action", "status"]).out);
    console.log(remote.ps("vbcable.ps1", ["-Action", "status"]).out);
    return EXIT.ok;
  }

  const lock = new RunLock(remote, log);
  try {
    await lock.acquire({ wait: args.wait });
  } catch (e) {
    await settle();
    log(e.message);
    return e instanceof LockBusy ? EXIT.busy : EXIT.runError;
  }
  const release = () => {
    try {
      lock.release();
    } catch (e) {
      log(`releasing the laptop lock: ${e.message}`);
    }
    remote.closeMaster();
  };
  onSignal = release; // replaced by run()'s clean-up while a run is going
  // Only with the lock held: any step on the laptop is then our own.
  onForce = () => remote.guardStep({ waitSec: 0, why: "forced by a second signal" });
  try {
    if (args.command === "selftest") {
      // The backup journal on fake folders (never the owner's); a step, so
      // it also exercises run-step.ps1 and the marker.
      const r = remote.ps("appdata-selftest.ps1", [], { check: false });
      console.log(r.out.trim());
      return r.code === 0 && !orphans.length ? EXIT.ok : EXIT.failed;
    }
    if (args.command === "restore") {
      const ok = restoreOwner(remote, log, { relaunch: true }).ok;
      restoreAudio(remote, log);
      for (const m of orphans) log(`clean-up error: ${m}`);
      return ok && !orphans.length ? EXIT.ok : EXIT.runError;
    }
    if (args.command === "vbcable-install") {
      installVbCable(remote, log);
      return EXIT.ok;
    }
    return await run(args, { runId, dir, log, remote, release, orphans });
  } finally {
    release();
  }
}

async function run(args, { runId, dir, log, remote, release, orphans }) {
  const results = { run: runId, installer: args.installerLocal ?? args.installer, host: args.host, started: new Date().toISOString(), pass: false, scenarios: {} };
  // A laptop step that had to be stopped (orphaned earlier, or after a
  // timeout or a forced stop) is a clean-up error: the run cannot pass.
  const orphan = (m) => (results.cleanup_errors = [...(results.cleanup_errors ?? []), `orphaned laptop step: ${m}`]);
  orphans.forEach(orphan);
  remote.onOrphan = orphan;
  const save = () => writeFileSync(join(dir, "results.json"), JSON.stringify(results, null, 2));
  const agent = new Agent(AGENT_PORT);
  const app = new App({ remote, log });
  // What has happened to the owner's data and app: decides the clean-up.
  const touched = { backupStarted: false, audioSaved: false };

  // The clean-up chain, run once: from the finally below, or from a signal.
  let cleaning = null;
  const cleanup = () => (cleaning ??= cleanupChain());
  onSignal = async (sig) => {
    results.interrupted = sig;
    log(`${sig}: interrupted; running the clean-up`);
    agent.stop(); // the scenario still awaiting the agent fails fast
    await cleanup();
    release();
  };

  async function cleanupChain() {
    await settle();
    // Each step on its own: one failing must not skip the others.
    const step = async (name, fn) => {
      try {
        await fn();
      } catch (e) {
        log(`clean-up step "${name}" failed: ${e.message}`);
        results.cleanup_errors = [...(results.cleanup_errors ?? []), `${name}: ${e.message}`];
      }
    };
    // Only the test profile's data, and only when the owner's is in the backup.
    if (touched.backupStarted) await step("collect test data", () => collectTestData(remote, app, dir, log, runId));
    await step("agent quit", () => agent.call("quit", {}, 5000, { force: true }).catch(() => {}));
    await step("stop leftover pads", () => {
      const r = remote.ps("pads.ps1", []);
      const kv = Remote.kv(r.out);
      if (kv.pads_stopped && kv.pads_stopped !== "0") log(`stopped ${kv.pads_stopped} leftover text pad(s)`);
      if (kv.pads_failed && kv.pads_failed !== "0") throw new Error(`could not stop ${kv.pads_failed} pad(s): ${r.out.trim()}`);
    });
    await step("close tunnels", () => remote.closeTunnels());
    if (touched.backupStarted && args.restore) {
      results.restored = false; // until restore says otherwise
      await step("restore owner data", () => {
        const r = restoreOwner(remote, log, { relaunch: true });
        results.restored = r.ok;
        if (!r.ok) throw new Error("restore did not confirm (see run.log)");
      });
    } else if (touched.backupStarted) {
      log("NOT restoring (--no-restore): run `tools/e2e-windows/run.sh restore` to put the owner's data back");
      results.restored = false;
    }
    if (touched.audioSaved) await step("restore audio defaults", () => restoreAudio(remote, log));
    await step("delete temp files", () => remote.ssh(`del /q ${remote.winPath("shots")}\\*.png`, { check: false }));
    await step("check leftover tasks", () => {
      results.leftover_tasks = remote.ssh('schtasks /query /fo csv | findstr /i "MurmurE2E"', { check: false }).out.trim();
    });
    results.pass = computePass(results, args);
    results.finished = new Date().toISOString();
    save();
    writeSummary(dir, results);
    log(`summary: ${join(dir, "summary.md")} (pass=${results.pass})`);
  }

  try {
    log(`results: ${dir}`);
    // The owner's install: the test build replaces it and stays installed.
    const owner = Remote.kv(remote.ps("appdata.ps1", ["-Action", "status"]).out);
    results.owner_version = owner.version ?? null;
    let installer = args.installer;
    if (args.installerLocal) {
      log(`copying ${basename(args.installerLocal)} to the laptop`);
      remote.mkdirp("installers");
      remote.scpTo([args.installerLocal], "installers/");
      installer = remote.winPath(`installers/${basename(args.installerLocal)}`);
    }
    checkNotDowngrade(remote, log, installer, results, args);

    const clips = generateClips(join(HERE, "clips"), join(cacheDir(), "clips"));
    mkdirSync(join(dir, "clips"), { recursive: true });
    for (const c of Object.values(clips)) copyFileSync(c.path, join(dir, "clips", basename(c.path)));
    remote.scpTo(Object.values(clips).map((c) => c.path), "clips/");
    for (const c of Object.values(clips)) c.remote = remote.winPath(`clips/${basename(c.path)}`);

    const cable = Remote.kv(remote.ps("vbcable.ps1", ["-Action", "status"]).out);
    if (!cable.driver) installVbCable(remote, log);
    // Refuses when an earlier run's saved defaults were never restored.
    log(remote.ps("audio.ps1", ["-Action", "save", "-File", remote.winPath(AUDIO_DEFAULTS)]).out.trim());
    touched.audioSaved = true;

    log("starting the interactive agent");
    remote.ps("itask.ps1", ["-Script", remote.winPath("agent.ps1"), "-Arguments", `-Port ${AGENT_PORT} -IdleMinutes 10`]);
    await remote.tunnel(AGENT_PORT);
    const ping = await agent.waitUp();
    log(`agent up in session ${ping.session}, desktop usable=${ping.desktop_usable}`);
    if (!ping.desktop_usable) throw new Error("the laptop is locked: SendInput cannot reach the desktop");

    log("backing up the owner's data");
    touched.backupStarted = true; // from here on, restore runs (it reads the journal)
    log(remote.ps("appdata.ps1", ["-Action", "backup", "-Id", runId]).out.trim());

    log(`installing ${installer} (it stays installed after the run)`);
    const inst = remote.ps("install.ps1", ["-Installer", installer], { timeoutMs: 300_000 }).out;
    writeFileSync(join(dir, "install.log"), inst);
    const ik = Remote.kv(inst);
    app.exe = ik.exe;
    results.version = ik.version;
    results.installer_autostarted = ik.autostarted === "True";
    log(`installed ${ik.version} at ${ik.exe} (installer started the app: ${ik.autostarted})`);
    log(remote.ps("appdata.ps1", ["-Action", "wipe"]).out.trim());

    await remote.tunnel(CDP_PORT);
    const ctx = { args, installer, dir, log, remote, agent, app, clips, results, cableIn: CABLE_IN, cableMic: CABLE_MIC, audioDefaults: remote.winPath(AUDIO_DEFAULTS), seedPatch: SEED_PATCH, seeded: false };

    for (const s of SCENARIOS) {
      if (!args.scenarios.includes(s.id)) continue;
      if (results.interrupted) break;
      log(`=== scenario ${s.id}: ${s.title}`);
      const t0 = Date.now();
      let r;
      try {
        if (s.needs === "fresh" && ctx.seeded) throw new Error("needs a fresh install but the profile is already seeded (run it first)");
        if (s.needs === "seeded" && !ctx.seeded) await seed(ctx);
        if (s.needs === "fresh" && !ctx.freshLaunched) {
          await app.launch();
          ctx.freshLaunched = true;
        }
        r = await s.run(ctx);
      } catch (e) {
        await settle();
        r = { status: results.interrupted ? "interrupted" : "error", summary: e.message, details: { stack: e.stack } };
      }
      r.title = s.title;
      r.seconds = Math.round((Date.now() - t0) / 1000);
      results.scenarios[s.id] = r;
      log(`--- ${s.id}: ${r.status.toUpperCase()} ${r.summary}`);
      save();
    }
  } catch (e) {
    await settle();
    if (!results.interrupted) {
      results.error = e.message;
      log(`RUN ERROR: ${e.stack}`);
    }
  } finally {
    await cleanup();
  }
  return exitCode(results, args);
}

/** Refuse an installer older than the owner's installed Murmur (unless --allow-downgrade). */
function checkNotDowngrade(remote, log, installer, results, args) {
  let v = installerVersion(installer);
  if (!v) v = Remote.kv(remote.ps("install.ps1", ["-Installer", installer, "-VersionOnly"]).out).installer_version || null;
  results.installer_version = v;
  const have = results.owner_version;
  log(`owner's installed Murmur: ${have ?? "none"}; installer: ${v ?? "unknown version"}`);
  if (!have || args.allowDowngrade) return;
  if (!v) throw new Error(`cannot tell the installer's version, and the owner has ${have} installed: pass --allow-downgrade to install it anyway`);
  if (compareVersions(v, have) < 0) throw new Error(`the installer (${v}) is older than the owner's installed ${have}: pass --allow-downgrade to replace it anyway`);
}

/**
 * Put the owner's data back (journal-driven, safe to repeat) and, if the
 * owner's app was running before the backup, start it again normally.
 */
function restoreOwner(remote, log, { relaunch }) {
  const out = remote.ps("appdata.ps1", ["-Action", "restore"]).out;
  log(out.trim());
  const kv = Remote.kv(out);
  const ok = /restored=|nothing pending/.test(out) && !/WARNING/.test(out);
  if (relaunch && kv.was_running === "True") relaunchOwnerApp(remote, log);
  return { ok, out };
}

/** Put the saved default audio devices and mute states back (no-op if none saved). */
function restoreAudio(remote, log) {
  log(remote.ps("audio.ps1", ["-Action", "restore", "-File", remote.winPath(AUDIO_DEFAULTS)]).out.trim());
}

/** Kill the app, put a test profile in place (owner prefs + patch + copied models), relaunch. */
async function seed(ctx) {
  const { remote, app, dir, log } = ctx;
  app.stop();
  const patch = join(dir, "seed-patch.json");
  writeFileSync(patch, JSON.stringify(ctx.seedPatch, null, 2));
  remote.scpTo([patch], "seed-patch.json");
  log(remote.ps("appdata.ps1", ["-Action", "seed", "-Patch", remote.winPath("seed-patch.json")], { timeoutMs: 600_000 }).out.trim());
  await app.launch();
  ctx.seeded = true;
}

/** The test profile's debug log, history and notes (synthetic dictations only). */
async function collectTestData(remote, app, dir, log, runId) {
  // Only while the owner's data is in a backup: then the app stopped here is
  // a test install and the files copied are test data.
  const pending = Remote.kv(remote.ps("appdata.ps1", ["-Action", "status"]).out).pending;
  if (!pending) return log("no pending backup: not copying anything (the live data would be the owner's)");
  if (pending !== runId) {
    log(`NOTE: the pending backup is ${pending}, not this run's (${runId}): the live test data copied now is from that earlier, unrestored run`);
  }
  app.stop();
  const out = join(dir, pending === runId ? "app-data" : `app-data-from-${pending}`);
  mkdirSync(out, { recursive: true });
  for (const [rel, name] of [
    ["debug/dictations.jsonl", "dictations.jsonl"],
    ["history.sqlite", "history.sqlite"],
    ["notes/scratchpad.txt", "scratchpad.txt"],
    ["notes/index.json", "notes-index.json"],
  ]) {
    try {
      remote.scpFrom(`AppData/Roaming/Murmur/${rel}`, join(out, name), { fromHome: true });
    } catch {
      /* not there */
    }
  }
  log(`test profile data copied to ${out}`);
}

function relaunchOwnerApp(remote, log) {
  const exe = Remote.kv(remote.ps("appdata.ps1", ["-Action", "status"]).out).exe;
  if (!exe) return log("could not find the installed exe to relaunch");
  remote.ps("itask.ps1", ["-Script", remote.winPath("launch-app.ps1"), "-Arguments", `-Exe ${exe} -DebugPort 0`, "-Wait"]);
  log("owner's app started again (no debug port)");
}

/** Download the official VB-Cable pack, copy it over, install it silently. */
function installVbCable(remote, log) {
  if (Remote.kv(remote.ps("vbcable.ps1", ["-Action", "status"]).out).driver) return log("VB-Cable already installed");
  log("installing VB-Cable (silent)");
  const unpacked = fetchVbCable(join(cacheDir(), "vbcable"));
  remote.mkdirp("vbcable");
  remote.scpTo(readdirSync(unpacked).map((f) => join(unpacked, f)), "vbcable/");
  log(remote.ps("vbcable.ps1", ["-Action", "install"], { timeoutMs: 300_000 }).out.trim());
}

main().then(
  (code) => {
    if (!signalled) process.exit(code);
  },
  (e) => {
    console.error(e);
    if (!signalled) process.exit(EXIT.runError);
  },
);
