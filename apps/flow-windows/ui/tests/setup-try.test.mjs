// Regression (D75, Windows setup on a clean install): the "Now try it" step
// stayed silent when a press could not start (no speech model, no server),
// because it only showed a finished dictation. It now says up front why it
// cannot work, and follows the pill (listening, working, the error). Runs
// on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { pillLine, readinessErrorLine, readinessLine } from "../dist/lib/setup-try.js";

test("ready: nothing to say before the user talks", () => {
  assert.equal(readinessLine({ ready: true, engine: "local", message: null }), null);
});

test("not ready: the backend's reason, in a bad line", () => {
  const l = readinessLine({
    ready: false,
    engine: null,
    message: "There's no speech model on this PC yet (nothing at C:\\models\\ggml-base.bin).",
  });
  assert.ok(l?.bad);
  assert.match(l.text, /^Dictation can't work yet\. There's no speech model on this PC yet/);
  assert.match(l.text, /Settings → Speech to text/);
});

test("not ready while the model downloads: wait, not an error", () => {
  const l = readinessLine({ ready: false, engine: null, message: "There's no speech model on this PC yet." }, true);
  assert.equal(l?.bad, false);
  assert.match(l.text, /still downloading/);
  assert.equal(readinessLine({ ready: true, engine: "remote", message: null }, true), null);
});

test("a failed check still says something", () => {
  const l = readinessErrorLine("The app is still starting.");
  assert.ok(l.bad);
  assert.match(l.text, /still starting/);
});

test("a press shows listening, then working", () => {
  assert.deepEqual(pillLine({ state: "active_ptt" }), { text: "Listening… let go when you're done.", bad: false });
  assert.equal(pillLine({ state: "active_locked" })?.bad, false);
  assert.equal(pillLine({ state: "processing" })?.bad, false);
});

test("a start failure on the pill reaches the step, one full stop", () => {
  const l = pillLine({ state: "error", message: "Speech model not found: x. Nothing is downloaded automatically." });
  assert.ok(l?.bad);
  assert.match(l.text, /^That didn't work: Speech model not found: x\. Nothing is downloaded automatically\. Try again/);
  assert.doesNotMatch(l.text, /\.\./);
});

test("a notice (no speech) is shown; resting keeps the current line", () => {
  assert.match(pillLine({ state: "notice", message: "No speech heard" })?.text ?? "", /^No speech heard\. Hold the shortcut/);
  assert.equal(pillLine({ state: "resting" }), null);
});
