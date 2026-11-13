// node --test tools/e2e-windows/test/  — RunLock against a fake laptop.
import assert from "node:assert/strict";
import { test } from "node:test";
import { HEARTBEAT_MS, LockBusy, RunLock, STALE_MS } from "../lib/lock.mjs";

const ROOT = "C:\\U\\murmur-e2e";

/** A fake laptop: just the dirs the lock uses, and the cmd lines it sends. */
class FakeRemote {
  constructor(clock) {
    this.clock = clock;
    this.dirs = new Map(); // path -> {owner: string|null, created: ms}
    this.down = false;
    this.calls = [];
    this.guards = [];
  }
  guardStep(o) {
    this.guards.push(o.why);
    return null;
  }
  winPath(rel = "") {
    return rel ? `${ROOT}\\${rel}` : ROOT;
  }
  ssh(cmd) {
    this.calls.push(cmd);
    if (this.down) return { code: 255, out: "ssh: connect to host: Connection timed out" };
    let m;
    if ((m = /^mkdir (\S+) && \(echo (.+)> \S+\\owner\.txt\) && ren \S+ (\S+)$/.exec(cmd))) {
      const [, tmp, owner, name] = m;
      if (this.dirs.has(tmp)) return { code: 1, out: "exists" };
      this.dirs.set(tmp, { owner, created: this.clock.t });
      return this.ren(tmp, name);
    }
    if ((m = /^if exist (\S+) rmdir \/s \/q \S+$/.exec(cmd))) {
      this.dirs.delete(m[1]);
      return { code: 0, out: "" };
    }
    if ((m = /^type (\S+)\\owner\.txt$/.exec(cmd))) {
      const d = this.dirs.get(m[1]);
      return d?.owner ? { code: 0, out: d.owner + "\r\n" } : { code: 1, out: "The system cannot find the file specified." };
    }
    if ((m = /^ren (\S+) (\S+)$/.exec(cmd))) return this.ren(m[1], m[2]);
    if ((m = /^rmdir \/s \/q (\S+)$/.exec(cmd))) {
      const had = this.dirs.delete(m[1]);
      return { code: had ? 0 : 2, out: "" };
    }
    throw new Error(`fake laptop: unexpected command ${cmd}`);
  }
  ren(from, name) {
    const to = `${ROOT}\\${name}`;
    if (!this.dirs.has(from) || this.dirs.has(to)) return { code: 1, out: "A duplicate file name exists, or the file cannot be found." };
    this.dirs.set(to, this.dirs.get(from));
    this.dirs.delete(from);
    return { code: 0, out: "" };
  }
  ps(script, args) {
    assert.equal(script, "lock.ps1");
    if (this.down) return { code: 255, out: "" };
    const d = this.dirs.get(args[1]);
    const last = d ? Math.max(d.created, d.beat ?? 0) : 0;
    return { code: 0, out: d ? `age_s=${Math.floor((this.clock.t - last) / 1000)}\r\n` : "age_s=\r\n" };
  }
}

function setup({ alive = () => true } = {}) {
  const beats = new Set(); // locks whose heartbeat is running
  const clock = { t: Date.parse("2026-11-13T10:00:00Z") };
  const remote = new FakeRemote(clock);
  const sleeps = [];
  const logs = [];
  const mk = (pid, host = "srv") =>
    new RunLock(remote, (s) => logs.push(s), {
      host,
      pid,
      now: () => clock.t,
      sleep: async (ms) => {
        sleeps.push(ms);
        clock.t += ms;
      },
      isAlive: alive,
      heartbeat: (lock) => {
        beats.add(lock);
        return () => beats.delete(lock);
      },
    });
  /** Let `ms` pass, the running heartbeats beating every minute. */
  const pass = (ms) => {
    for (let left = ms; left > 0; left -= HEARTBEAT_MS) {
      clock.t += Math.min(HEARTBEAT_MS, left);
      for (const l of beats) if (remote.dirs.get(l.dir)?.owner?.startsWith(l.me + " ")) remote.dirs.get(l.dir).beat = clock.t;
    }
  };
  return { clock, remote, sleeps, logs, mk, beats, pass, lockDir: `${ROOT}\\lock` };
}

test("take and release: the owner is written with the lock", async () => {
  const { remote, mk, lockDir } = setup();
  const a = mk(100);
  await a.acquire({ wait: false });
  assert.match(remote.dirs.get(lockDir).owner, /^srv 100 /);
  a.release();
  assert.equal(remote.dirs.has(lockDir), false);
});

test("no wait: a live lock gives LockBusy at once", async () => {
  const { mk, sleeps } = setup();
  await mk(100).acquire();
  await assert.rejects(mk(200).acquire({ wait: false }), LockBusy);
  assert.deepEqual(sleeps, []);
});

test("wait: backs off (2 s doubling to 20 s), honours maxWaitMs", async () => {
  const { mk, sleeps } = setup();
  await mk(100).acquire();
  await assert.rejects(mk(200).acquire({ maxWaitMs: 60_000 }), /gave up waiting/);
  assert.deepEqual(sleeps.slice(0, 5), [2000, 4000, 8000, 16000, 20000]);
  assert.ok(sleeps.reduce((a, b) => a + b, 0) <= 60_000);
});

test("wait: gets the lock once the holder releases it", async () => {
  const { mk, clock } = setup();
  const a = mk(100);
  await a.acquire();
  const b = mk(200);
  const p = b.acquire();
  await Promise.resolve();
  a.release();
  await p;
  assert.equal(b.held, true);
  assert.ok(clock.t >= 0);
});

test("a dead holder on this host is stale and broken", async () => {
  const { mk, remote, lockDir, logs } = setup({ alive: (pid) => pid !== 100 });
  await mk(100).acquire();
  await mk(200).acquire({ wait: false });
  assert.match(remote.dirs.get(lockDir).owner, /^srv 200 /);
  assert.ok(logs.some((l) => /breaking a stale/.test(l)));
});

test("a holder elsewhere keeps its lock while it beats, however long it runs", async () => {
  const { mk, pass, beats, remote, lockDir } = setup();
  const a = mk(100, "other");
  await a.acquire();
  assert.equal(beats.size, 1);
  pass(5 * 3600_000); // a 5 h run
  await assert.rejects(mk(200).acquire({ wait: false }), LockBusy);
  a.stopBeat(); // killed without a release: no more heartbeats
  pass(STALE_MS - 60_000);
  await assert.rejects(mk(200).acquire({ wait: false }), LockBusy);
  pass(2 * 60_000);
  await mk(200).acquire({ wait: false });
  assert.match(remote.dirs.get(lockDir).owner, /^srv 200 /);
});

test("release stops the heartbeat", async () => {
  const { mk, beats } = setup();
  const a = mk(100);
  await a.acquire();
  a.release();
  assert.equal(beats.size, 0);
});

test("breaking a stale lock first stops a step its owner left running", async () => {
  const { mk, remote } = setup({ alive: (pid) => pid !== 100 });
  await mk(100).acquire();
  await mk(200).acquire({ wait: false });
  assert.deepEqual(remote.guards, ["before breaking a stale lock"]);
});

test("no stale break, no step guard", async () => {
  const { mk, remote } = setup();
  await mk(100).acquire();
  await assert.rejects(mk(200).acquire({ wait: false }), LockBusy);
  assert.deepEqual(remote.guards, []);
});

test("an ownerless lock dir is busy while young, stale after 10 min", async () => {
  const { mk, clock, remote, lockDir } = setup();
  remote.dirs.set(lockDir, { owner: null, created: clock.t });
  await assert.rejects(mk(200).acquire({ wait: false }), LockBusy);
  clock.t += STALE_MS + 1000;
  await mk(200).acquire({ wait: false });
  assert.match(remote.dirs.get(lockDir).owner, /^srv 200 /);
});

test("an unreachable laptop is an error, not a stale lock", async () => {
  const { mk, remote } = setup();
  remote.down = true;
  const e = await mk(200).acquire().catch((x) => x);
  assert.ok(e instanceof Error && !(e instanceof LockBusy));
  assert.match(e.message, /unreachable/);
  assert.equal(remote.calls.filter((c) => c.startsWith("rmdir")).length, 0);
});

test("release leaves someone else's lock alone", async () => {
  const { mk, remote, lockDir } = setup({ alive: (pid) => pid !== 100 });
  const a = mk(100);
  await a.acquire();
  await mk(200).acquire({ wait: false }); // broke a's lock as stale
  a.release();
  assert.match(remote.dirs.get(lockDir).owner, /^srv 200 /);
});
