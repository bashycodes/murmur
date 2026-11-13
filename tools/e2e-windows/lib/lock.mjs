// The laptop-side run lock: one harness run (or restore) at a time, from any
// machine. run.sh also takes a flock on this server, so local runs queue
// before they get here; this lock catches runs from elsewhere and is the
// one that matters if two servers share the laptop.
//
// The lock is the directory murmur-e2e\lock holding owner.txt =
// "<host> <pid> <iso time>". It is taken atomically WITH its owner: build
// lock.tmp-<host>-<pid>\owner.txt, then rename it to `lock` (cmd's ren
// fails when `lock` exists).
//
// While it runs, the holder refreshes lock\heartbeat every minute (a
// detached local loop, so it beats through a long install or a clean-up).
// A lock is stale when its owner is a dead process on this host, or when
// it has had no heartbeat for 10 min (age = since the newest of the dir's
// creation, owner.txt and heartbeat; ps/lock.ps1). A run of any length
// keeps its lock; a killed one loses it 10 min later. A stale lock is broken
// by renaming it aside (only one breaker can win the rename); a step its
// owner left running on the laptop is stopped (Remote.guardStep), then the
// lock is deleted.
import { spawn } from "node:child_process";
import { hostname } from "node:os";

export const STALE_MS = 10 * 60_000;
export const HEARTBEAT_MS = 60_000;
const FIRST_DELAY_MS = 2_000;
const MAX_DELAY_MS = 20_000;

/** The lock is held by another live run (and the caller did not wait, or waited too long). */
export class LockBusy extends Error {}

const realSleep = (ms) => new Promise((r) => setTimeout(r, ms));
function pidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === "EPERM";
  }
}

/**
 * Refresh lock\heartbeat every minute from a detached local loop (it keeps
 * beating while node is blocked in a long ssh) that ends with this process
 * or when stopped. It only writes while owner.txt is still ours. Returns stop().
 */
function sshHeartbeat(lock) {
  const { remote } = lock;
  const beat = `findstr /b /c:"${lock.me} " ${lock.dir}\\owner.txt >nul && (echo %TIME%> ${lock.dir}\\heartbeat)`;
  const loop = 'while sleep "$2" && kill -0 "$1" 2>/dev/null; do timeout 45 ssh "${@:5}" "$3" "$4" >/dev/null 2>&1; done';
  const p = spawn("bash", ["-c", loop, "heartbeat", String(process.pid), String(HEARTBEAT_MS / 1000), remote.host, beat, ...remote.sshOpts],
    { detached: true, stdio: "ignore" });
  p.unref();
  return () => {
    try {
      process.kill(-p.pid);
    } catch {
      /* already gone */
    }
  };
}

export class RunLock {
  /**
   * @param {import("./remote.mjs").Remote} remote
   * @param {(s: string) => void} log
   * @param {object} [deps] for tests: host, pid, now(), sleep(ms), isAlive(pid), heartbeat(lock) -> stop()
   */
  constructor(remote, log, { host = hostname(), pid = process.pid, now = Date.now, sleep = realSleep, isAlive = pidAlive, heartbeat = sshHeartbeat } = {}) {
    this.remote = remote;
    this.log = log;
    this.host = host;
    this.pid = pid;
    this.now = now;
    this.sleep = sleep;
    this.isAlive = isAlive;
    this.heartbeat = heartbeat;
    this.stopBeat = null;
    this.dir = remote.winPath("lock");
    this.me = `${host} ${pid}`;
    this.held = false;
  }

  /** ssh that throws when the laptop cannot be reached (ssh exit 255 or a timeout). */
  cmd(c) {
    const r = this.remote.ssh(c, { check: false, timeoutMs: 60_000 });
    if (r.code === 255 || r.code === null || r.code === undefined) throw new Error(`the laptop is unreachable (ssh ${r.code ?? "timeout"}): ${r.out.trim()}`);
    return r;
  }

  /** Owner of the lock dir `dir`: {host, pid, at, text}, or null when it has no owner.txt. */
  owner(dir = this.dir) {
    const r = this.cmd(`type ${dir}\\owner.txt`);
    if (r.code !== 0) return null;
    const text = r.out.trim();
    const [host, pid, at] = text.split(/\s+/);
    return { host, pid: Number(pid), at, text };
  }

  /** Time since the lock's last sign of life (heartbeat) in ms, or null when there is no lock dir. */
  ageMs() {
    const r = this.remote.ps("lock.ps1", ["-Path", this.dir], { check: false, timeoutMs: 60_000 });
    if (r.code === 255 || r.code === null || r.code === undefined) throw new Error(`the laptop is unreachable (ssh ${r.code ?? "timeout"})`);
    const m = /age_s=(\d+)/.exec(r.out);
    return m ? Number(m[1]) * 1000 : null;
  }

  tryTake() {
    const tmp = `${this.dir}.tmp-${this.host}-${this.pid}`;
    const r = this.cmd(`mkdir ${tmp} && (echo ${this.me} ${new Date(this.now()).toISOString()}> ${tmp}\\owner.txt) && ren ${tmp} lock`);
    if (r.code === 0) {
      this.held = true;
      this.stopBeat = this.heartbeat(this);
      return true;
    }
    this.cmd(`if exist ${tmp} rmdir /s /q ${tmp}`);
    return false;
  }

  /**
   * Rename a stale lock aside and delete it; if it changed hands meanwhile,
   * put it back. Once it is ours, a step its owner left running on the
   * laptop is waited for, then stopped, before the lock goes.
   */
  breakStale(seen) {
    const aside = `lock.stale-${this.host}-${this.pid}`;
    if (this.cmd(`ren ${this.dir} ${aside}`).code !== 0) return; // gone or someone else broke it
    const path = this.remote.winPath(aside);
    const now = this.owner(path);
    if ((now?.text ?? null) !== (seen?.text ?? null)) {
      // A fresh lock was taken between our check and the rename: give it back.
      this.log("the lock changed hands while breaking it: putting it back");
      this.cmd(`ren ${path} lock`);
      return;
    }
    this.remote.guardStep({ why: "before breaking a stale lock" });
    this.cmd(`rmdir /s /q ${path}`);
  }

  /**
   * Take the lock. Without `wait`, throw LockBusy at once when another live
   * run holds it; with it, back off (2 s doubling to 20 s) until it is free
   * or `maxWaitMs` has passed. An unreachable laptop throws a plain Error.
   */
  async acquire({ wait = true, maxWaitMs = 2 * 3600_000 } = {}) {
    const end = this.now() + maxWaitMs;
    let delay = FIRST_DELAY_MS;
    let said = null;
    for (;;) {
      if (this.tryTake()) return;
      const o = this.owner();
      const dead = o && o.host === this.host && !this.isAlive(o.pid);
      const age = dead ? null : this.ageMs();
      const stale = dead || (age !== null && age > STALE_MS);
      const what = o ? o.text : age === null ? "released meanwhile" : "no owner";
      if (stale) {
        this.log(`breaking a stale laptop lock (${what}; ${dead ? "its process is gone" : `no heartbeat for ${Math.round(age / 1000)} s`})`);
        this.breakStale(o);
      } else if (o || what !== "released meanwhile") {
        if (!wait) throw new LockBusy(`the laptop is busy with another e2e run (${what})`);
        if (said !== what) this.log(`laptop busy (${what}); waiting`);
        said = what;
      }
      if (this.now() + delay > end) throw new LockBusy(`gave up waiting for the laptop lock after ${Math.round(maxWaitMs / 1000)} s (${what})`);
      await this.sleep(delay);
      delay = Math.min(delay * 2, MAX_DELAY_MS);
    }
  }

  release() {
    if (!this.held) return;
    this.held = false;
    this.stopBeat?.();
    this.stopBeat = null;
    const o = this.owner();
    if (o && `${o.host} ${o.pid}` === this.me) this.cmd(`rmdir /s /q ${this.dir}`);
  }
}
