// node --test tools/e2e-windows/test/  — scenario h's grading.
import assert from "node:assert/strict";
import { test } from "node:test";
import { isStateChanging } from "../lib/stepmark.mjs";
import { SCENARIOS } from "../scenarios/index.mjs";
import { gradeDialog, gradePhase, installerPath } from "../scenarios/h-uninstall-data.mjs";

const gone = { murmur: "absent", local: "absent", installed: "False" };
const kept = { murmur: "present", local: "present", installed: "False" };

test("ticked: the app folder and Tauri's folder must be gone", () => {
  assert.deepEqual(gradePhase("ticked", gone), []);
  // The bug: Tauri's folder went, ours stayed.
  assert.deepEqual(gradePhase("ticked", { ...gone, murmur: "present" }), ["ticked: murmur=present, want absent"]);
  assert.equal(gradePhase("ticked", { ...gone, installed: "True" }).length, 1);
});

test("unticked and installs over: the data stays", () => {
  assert.deepEqual(gradePhase("unticked", kept), []);
  // Tauri's own folder is not graded when unticked.
  assert.deepEqual(gradePhase("unticked", { ...kept, local: "absent" }), []);
  assert.equal(gradePhase("unticked", gone).length, 1);
  for (const p of ["installed", "silent-over", "update"]) {
    assert.deepEqual(gradePhase(p, { ...kept, installed: "True" }), []);
    assert.equal(gradePhase(p, { ...kept, installed: "True", murmur: "absent" }).length, 1);
    assert.equal(gradePhase(p, kept).length, 1, `${p}: must still be installed`);
  }
  assert.deepEqual(gradePhase("unticked", {}), ["unticked: murmur=?, want present", "unticked: installed=?, want False"]);
  assert.throws(() => gradePhase("nope", {}), /unknown phase/);
});

test("the dialog: the box starts unticked and is as asked when uninstalling", () => {
  const d = { checkbox_before: "Off", checkbox_after: "On", entry_removed: true };
  assert.deepEqual(gradeDialog("ticked", d), []);
  assert.deepEqual(gradeDialog("unticked", { ...d, checkbox_after: "Off" }), []);
  assert.equal(gradeDialog("ticked", { ...d, checkbox_after: "Off" }).length, 1);
  assert.equal(gradeDialog("unticked", d).length, 1);
  assert.equal(gradeDialog("ticked", { ...d, checkbox_before: "On" }).length, 1);
  assert.equal(gradeDialog("ticked", { ...d, entry_removed: false }).length, 1);
});

test("installer path for the agent: names are in Downloads", () => {
  assert.equal(installerPath("Murmur_1.3.0_x64-setup.exe"), "%USERPROFILE%\\Downloads\\Murmur_1.3.0_x64-setup.exe");
  assert.equal(installerPath("C:\\x\\M.exe"), "C:\\x\\M.exe");
});

test("h runs last and its check is read-only", () => {
  assert.equal(SCENARIOS.at(-1).id, "h");
  assert.equal(isStateChanging("uninstall-data.ps1", ["-Action", "check"]), false);
  assert.equal(isStateChanging("uninstall-data.ps1", ["-Action", "seed", "-Id", "x"]), true);
});
