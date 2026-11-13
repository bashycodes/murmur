// node --test tools/e2e-windows/test/  — the run's pass flag and exit code.
import assert from "node:assert/strict";
import { test } from "node:test";
import { EXIT, compareVersions, computePass, exitCode, installerVersion } from "../lib/verdict.mjs";

const ok = { status: "pass" };
const run = (o = {}) => ({ scenarios: { a: ok, c: ok }, restored: true, ...o });
const R = { restore: true };
const NR = { restore: false };

test("pass needs every scenario to pass", () => {
  assert.equal(computePass(run(), R), true);
  assert.equal(computePass(run({ scenarios: { a: ok, c: { status: "fail" } } }), R), false);
  assert.equal(computePass(run({ scenarios: { a: ok, c: { status: "error" } } }), R), false);
  assert.equal(computePass(run({ scenarios: { a: ok, c: { status: "aborted" } } }), R), false);
  assert.equal(computePass(run({ scenarios: {} }), R), false);
});

test("pass needs a confirmed restore (not just an unset one)", () => {
  assert.equal(computePass(run({ restored: false }), R), false);
  assert.equal(computePass(run({ restored: undefined }), R), false);
  // --no-restore: no restore expected
  assert.equal(computePass(run({ restored: false }), NR), true);
});

test("pass fails on clean-up errors, a run error or an interruption", () => {
  assert.equal(computePass(run({ cleanup_errors: ["stop leftover pads: x"] }), R), false);
  assert.equal(computePass(run({ cleanup_errors: [] }), R), true);
  assert.equal(computePass(run({ error: "boom" }), R), false);
  assert.equal(computePass(run({ interrupted: "SIGINT" }), R), false);
});

test("exit codes", () => {
  assert.equal(exitCode({ ...run(), pass: true }, R), EXIT.ok);
  assert.equal(exitCode({ ...run(), pass: false }, R), EXIT.failed);
  assert.equal(exitCode({ ...run(), error: "boom", pass: false }, R), EXIT.runError);
  assert.equal(exitCode({ ...run(), interrupted: "SIGTERM", pass: false }, R), EXIT.interrupted);
  // --no-restore has its own code, whatever the verdict
  assert.equal(exitCode({ ...run(), pass: true }, NR), EXIT.noRestore);
  assert.equal(exitCode({ ...run(), pass: false }, NR), EXIT.noRestore);
  assert.deepEqual([EXIT.ok, EXIT.runError, EXIT.usage, EXIT.failed, EXIT.busy, EXIT.noRestore, EXIT.interrupted], [0, 1, 2, 3, 4, 5, 130]);
});

test("versions", () => {
  assert.equal(installerVersion("Murmur_1.3.0_x64-setup_hybrid.exe"), "1.3.0");
  assert.equal(installerVersion("C:\\x\\installers\\Murmur_1.10.2_x64-setup.exe"), "1.10.2");
  assert.equal(installerVersion("setup.exe"), null);
  assert.ok(compareVersions("1.3.0", "1.10.0") < 0);
  assert.ok(compareVersions("1.4.0", "1.3.9") > 0);
  assert.equal(compareVersions("1.3.0", "1.3.0"), 0);
});
