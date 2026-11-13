// SSH / scp to the Windows laptop, PowerShell scripts there, and the SSH
// tunnels to the app's CDP port and the interactive agent.
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { cacheDir } from "./paths.mjs";
import { encodeParams, isStateChanging, parseGuard } from "./stepmark.mjs";

const CACHE = cacheDir();
/**
 * Every ssh / scp runs under ssh-shield.sh, in its own session: a Ctrl+C on
 * the terminal never cuts a laptop step in half (an install or restore
 * killed mid-way is worse than one that ends). A second Ctrl+C within 3 s
 * makes the shield stop the ssh and exit FORCED; a spawnSync timeout
 * (SIGTERM) makes it stop the ssh too. Either way the laptop step may live
 * on, orphaned: ssh() then stops it through its step marker (guardStep).
 */
const SHIELD = fileURLToPath(new URL("./ssh-shield.sh", import.meta.url));
const FORCED = 86;
const shielded = (cmd, args, opts) => spawnSync("bash", [SHIELD, cmd, ...args], { encoding: "utf8", ...opts });

/** A laptop step stopped by a forced Ctrl+C or a timeout. */
export class StepStopped extends Error {}

export class Remote {
  /**
   * @param {object} o
   * @param {string} o.host  ssh destination, e.g. user@host
   * @param {string} o.dir   work dir on the laptop, relative to the user profile
   * @param {(s: string) => void} o.log
   * @param {(msg: string) => void} [o.onOrphan] a laptop step had to be stopped (a clean-up error)
   */
  constructor({ host, dir, log, onOrphan = () => {} }) {
    this.host = host;
    this.dir = dir;
    this.log = log;
    this.onOrphan = onOrphan;
    mkdirSync(CACHE, { recursive: true });
    // One multiplexed connection: each later ssh/scp skips the handshake.
    this.sshOpts = [
      "-o", "BatchMode=yes",
      "-o", "ConnectTimeout=15",
      "-o", "ControlMaster=auto",
      "-o", `ControlPath=${CACHE}/ssh-%C`,
      "-o", "ControlPersist=120",
    ];
    this.tunnels = [];
  }

  /** Absolute Windows path of `rel` inside the work dir. */
  winPath(rel = "") {
    if (!this.profile) {
      // %USERPROFILE% of the ssh user, asked once (it need not be C:\Users\<user>).
      this.profile = this.ssh("echo %USERPROFILE%").out.trim().split(/\r?\n/).at(-1).trim();
      if (!/^[A-Za-z]:\\/.test(this.profile)) throw new Error(`unexpected USERPROFILE: ${this.profile}`);
    }
    return `${this.profile}\\${this.dir}${rel ? "\\" + rel.replaceAll("/", "\\") : ""}`;
  }

  /**
   * Run `cmd` on the laptop. A forced Ctrl+C or a timeout stops the local
   * ssh; with `step` (a step that may outlive it there), the laptop step is
   * then waited for briefly and stopped (guardStep). Both throw StepStopped.
   */
  ssh(cmd, { timeoutMs = 300_000, check = true, step = false } = {}) {
    const r = shielded("ssh", [...this.sshOpts, this.host, cmd], { timeout: timeoutMs, maxBuffer: 64 << 20 });
    const out = (r.stdout ?? "") + (r.stderr ?? "");
    const forced = r.status === FORCED;
    const timedOut = !forced && (r.error?.code === "ETIMEDOUT" || r.status === null);
    if (forced || timedOut) {
      const why = forced ? "stopped by a second Ctrl+C" : `timed out after ${Math.round(timeoutMs / 1000)} s`;
      if (step) {
        this.log(`laptop step ${why}: stopping what is left of it on the laptop`);
        this.guardStep({ waitSec: forced ? 0 : 30, why });
      }
      throw new StepStopped(`ssh ${why}: ${cmd}\n${out}`);
    }
    if (check && r.status !== 0) throw new Error(`ssh failed (${r.status ?? r.signal}): ${cmd}\n${out}`);
    return { code: r.status, out };
  }

  /**
   * Run tools/e2e-windows/ps/<script> on the laptop (non-interactive session).
   * A state-changing script runs as a step, through run-step.ps1 (marker and
   * guard, lib/stepmark.mjs); `marker: false` runs it directly.
   */
  ps(script, args = [], opts = {}) {
    const { marker = isStateChanging(script, args), ...rest } = opts;
    const psExe = "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File";
    if (marker) {
      const r = this.ssh(`${psExe} ${this.winPath("run-step.ps1")} -Script ${script} -ParamsB64 ${encodeParams(args)}`, { ...rest, step: true });
      this.noteGuard(parseGuard(r.out), `before ${script}`);
      return r;
    }
    const a = args.map((x) => (/[\s"]/.test(x) ? `"${x.replaceAll('"', '\\"')}"` : x)).join(" ");
    return this.ssh(`${psExe} ${this.winPath(script)} ${a}`, rest);
  }

  /**
   * Is a harness step still running on the laptop (an orphan of a killed
   * ssh)? Wait for it up to `waitSec`, then stop its process tree. A step
   * that had to be stopped is reported through onOrphan. Returns parseGuard().
   */
  guardStep({ waitSec = 90, why = "check" } = {}) {
    const r = this.ps("stepguard.ps1", ["-WaitSec", String(waitSec), "-Kill"], { marker: false, check: false, timeoutMs: (waitSec + 60) * 1000 });
    const g = parseGuard(r.out);
    this.noteGuard(g, why);
    return g;
  }

  noteGuard(g, why) {
    if (!g || g.state === "none" || g.state === "stale") return;
    const what = `${g.script ?? "?"} (pid ${g.pid ?? "?"})`;
    if (g.orphan) {
      const msg = `laptop step ${what} was still running (${why}): ${g.state === "killed" ? "stopped it" : "could NOT stop it"}`;
      this.log(msg);
      this.onOrphan(msg);
    } else this.log(`waited ${g.waited} s for the laptop step ${what} to finish (${why})`);
  }

  /** Parse `key=value` lines of a script's output. */
  static kv(out) {
    const m = {};
    for (const line of out.split(/\r?\n/)) {
      const i = line.indexOf("=");
      if (i > 0 && /^[a-z_]+$/.test(line.slice(0, i))) m[line.slice(0, i)] = line.slice(i + 1).trim();
    }
    return m;
  }

  scpTo(localPaths, remoteRel) {
    const dest = `${this.host}:${this.dir}/${remoteRel}`.replace(/\/$/, "/");
    const r = shielded("scp", [...this.sshOpts, "-q", ...localPaths, dest]);
    if (r.status !== 0) throw new Error(`scp to ${dest} failed: ${r.stderr}`);
  }

  /** Copy a file from the work dir (or, with fromHome, from the user profile). */
  scpFrom(remoteRel, localPath, { fromHome = false } = {}) {
    const src = fromHome ? remoteRel : `${this.dir}/${remoteRel}`;
    const r = shielded("scp", [...this.sshOpts, "-q", `${this.host}:${src}`, localPath]);
    if (r.status !== 0) throw new Error(`scp from ${remoteRel} failed: ${r.stderr}`);
  }

  /** Create `rel` inside the work dir ("" = the work dir) unless it exists. */
  mkdirp(rel = "") {
    const p = this.winPath(rel);
    this.ssh(`if not exist ${p} mkdir ${p}`);
  }

  /** Copy the PowerShell side (ps/*) to the work dir. */
  sync(psDir) {
    for (const sub of ["", "clips", "shots"]) this.mkdirp(sub);
    const files = readdirSync(psDir).map((f) => join(psDir, f));
    this.scpTo(files, "");
  }

  /** `ssh -N -L local:127.0.0.1:remote`; closed by closeTunnels(). */
  async tunnel(port) {
    // Own connection (not the master): killing it closes just this tunnel.
    const p = spawn("ssh", ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ExitOnForwardFailure=yes",
      "-N", "-L", `${port}:127.0.0.1:${port}`, this.host], { stdio: "ignore" });
    this.tunnels.push(p);
    await new Promise((r) => setTimeout(r, 1500));
    if (p.exitCode !== null) throw new Error(`tunnel on ${port} failed (port in use?)`);
    return p;
  }

  closeTunnels() {
    for (const p of this.tunnels) p.kill();
    this.tunnels = [];
  }

  closeMaster() {
    spawnSync("ssh", [...this.sshOpts, "-O", "exit", this.host], { stdio: "ignore" });
  }
}
