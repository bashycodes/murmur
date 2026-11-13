// node --test tools/e2e-windows/test/  — laptop steps: marker logic, the
// signal decision, the ssh shield, and f's status.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { FORCE_WINDOW_MS, encodeParams, isStateChanging, parseGuard, signalAction, toNamedParams } from "../lib/stepmark.mjs";
import { languageStatus } from "../scenarios/f-language.mjs";

test("toNamedParams: values, switches, values with spaces", () => {
  assert.deepEqual(toNamedParams(["-Action", "restore", "-Keep", "-File", "C:\\x y\\a.json"]), { Action: "restore", Keep: true, File: "C:\\x y\\a.json" });
  assert.deepEqual(toNamedParams(["-Script", "C:\\a.ps1", "-Arguments", "-Port 9224 -IdleMinutes 10", "-Wait"]), { Script: "C:\\a.ps1", Arguments: "-Port 9224 -IdleMinutes 10", Wait: true });
  assert.deepEqual(toNamedParams([]), {});
  assert.throws(() => toNamedParams(["restore"]), /positional/);
});

test("encodeParams: base64 of the JSON run-step.ps1 decodes", () => {
  const b = encodeParams(["-Action", "seed", "-Patch", "C:\\p.json"]);
  assert.match(b, /^[A-Za-z0-9+/=]+$/);
  assert.deepEqual(JSON.parse(Buffer.from(b, "base64").toString("utf8")), { Action: "seed", Patch: "C:\\p.json" });
});

test("isStateChanging: read-only calls skip the marker", () => {
  assert.equal(isStateChanging("lock.ps1", ["-Path", "x"]), false);
  assert.equal(isStateChanging("stepguard.ps1", []), false);
  assert.equal(isStateChanging("appdata.ps1", ["-Action", "status"]), false);
  assert.equal(isStateChanging("appdata.ps1", ["-Action", "ownerprefs", "-Keys", "a"]), false);
  assert.equal(isStateChanging("vbcable.ps1", ["-Action", "status"]), false);
  assert.equal(isStateChanging("install.ps1", ["-Installer", "x", "-VersionOnly"]), false);
  for (const a of ["restore", "backup", "wipe", "seed"]) assert.equal(isStateChanging("appdata.ps1", ["-Action", a]), true);
  assert.equal(isStateChanging("vbcable.ps1", ["-Action", "install"]), true);
  assert.equal(isStateChanging("audio.ps1", ["-Action", "restore", "-Keep"]), true);
  assert.equal(isStateChanging("install.ps1", ["-Installer", "x"]), true);
  assert.equal(isStateChanging("pads.ps1", []), true);
  assert.equal(isStateChanging("appdata-selftest.ps1", []), true);
});

test("parseGuard: the guard's report", () => {
  assert.equal(parseGuard("restored=1\r\n"), null);
  assert.deepEqual(parseGuard("step_guard=none\r\n"), { state: "none", script: null, pid: null, waited: 0, orphan: false });
  const k = parseGuard("step_script=install.ps1\r\nstep_pid=4242\r\nstep_waited_s=90\r\nstep_guard=killed\r\nother=1");
  assert.deepEqual(k, { state: "killed", script: "install.ps1", pid: 4242, waited: 90, orphan: true });
  assert.equal(parseGuard("step_script=a\r\nstep_pid=1\r\nstep_guard=alive").orphan, true);
  assert.equal(parseGuard("step_script=a\r\nstep_pid=1\r\nstep_waited_s=3\r\nstep_guard=finished").orphan, false);
  assert.equal(parseGuard("step_script=a\r\nstep_pid=1\r\nstep_guard=stale").orphan, false);
});

test("signalAction: first cleans up, second within 3 s forces, then ignore", () => {
  assert.equal(signalAction(null, 1000, false), "cleanup");
  assert.equal(signalAction(1000, 1000 + FORCE_WINDOW_MS, false), "force");
  assert.equal(signalAction(1000, 1000 + FORCE_WINDOW_MS + 1, false), "ignore");
  assert.equal(signalAction(1000, 1500, true), "ignore");
});

const SHIELD = fileURLToPath(new URL("../lib/ssh-shield.sh", import.meta.url));
/** Run `sh -c script` under the shield, sending it `signals` at the given ms. */
function shield(script, signals) {
  return new Promise((resolve) => {
    const p = spawn("bash", [SHIELD, "sh", "-c", script], { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    p.stdout.on("data", (d) => (out += d));
    p.stderr.on("data", (d) => (out += d));
    for (const [sig, at] of signals) setTimeout(() => p.kill(sig), at);
    p.on("exit", (code) => resolve({ code, out }));
  });
}

test("ssh shield: one Ctrl+C lets the command finish with its own status", async () => {
  const r = await shield("sleep 1; echo done; exit 3", [["SIGINT", 300]]);
  assert.equal(r.code, 3);
  assert.match(r.out, /letting the laptop step finish/);
  assert.match(r.out, /done/);
});

test("ssh shield: a second Ctrl+C within 3 s stops it (exit 86)", async () => {
  const r = await shield("sleep 2; echo done", [["SIGINT", 200], ["SIGINT", 500]]);
  assert.equal(r.code, 86);
  assert.match(r.out, /stopping the laptop step/);
  assert.doesNotMatch(r.out, /done/);
});

test("ssh shield: SIGTERM (a spawnSync timeout) stops it (exit 124)", async () => {
  const r = await shield("sleep 2; echo done", [["SIGTERM", 200]]);
  assert.equal(r.code, 124);
  assert.doesNotMatch(r.out, /done/);
});

test("f status: WRONG beats aborted; aborted only when nothing ran wrong", () => {
  const ok = { graded: true, ok: true };
  const wrong = { graded: true, ok: false };
  const abort = { graded: true, ok: false, aborted: true };
  const mixed = { graded: false, ok: null };
  assert.equal(languageStatus([ok, ok, mixed]), "pass");
  assert.equal(languageStatus([wrong, abort, mixed]), "fail");
  assert.equal(languageStatus([abort, ok, mixed]), "aborted");
  assert.equal(languageStatus([wrong, ok]), "fail");
});
