// Setup's speech step recommends This PC on a new install (step 1 promises it)
// and "Your server" only once one is saved (UX review on 1.3.0, amends D73).
// An engine already in use stays picked. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { initialEngine, recommendedEngine } from "../dist/lib/setup-engine.js";

const build = (...on) => (e) => on.includes(e);
const full = build("pc", "server", "cloud");

test("new install, no server saved: This PC is recommended", () => {
  assert.equal(recommendedEngine(full, false), "pc");
});

test("a saved server: Your server is recommended", () => {
  assert.equal(recommendedEngine(full, true), "server");
});

test("build without a server engine: This PC, even with an address saved", () => {
  assert.equal(recommendedEngine(build("pc", "cloud"), true), "pc");
  assert.equal(recommendedEngine(build("cloud"), false), null);
});

test("build without This PC recommends a saved server only", () => {
  assert.equal(recommendedEngine(build("server", "cloud"), false), null);
  assert.equal(recommendedEngine(build("server", "cloud"), true), "server");
});

test("clean install (this PC on by default, never chosen): This PC is pre-selected", () => {
  assert.equal(initialEngine("pc", false, full, false), "pc");
  assert.equal(initialEngine(null, false, full, false), "pc");
});

test("clean install with a server address saved: Your server is pre-selected", () => {
  assert.equal(initialEngine("pc", false, full, true), "server");
});

test("a server or OpenRouter already in use stays picked", () => {
  assert.equal(initialEngine("server", false, full, true), "server");
  assert.equal(initialEngine("cloud", true, full, false), "cloud");
});

test("setup re-run after choosing This PC keeps This PC", () => {
  assert.equal(initialEngine("pc", true, full, true), "pc");
});

test("builds without This PC fall back to Your server, then the cloud", () => {
  assert.equal(initialEngine(null, false, build("server", "cloud"), false), "server");
  assert.equal(initialEngine(null, false, build("cloud"), false), "cloud");
  assert.equal(initialEngine("pc", false, build("pc", "cloud"), false), "pc");
});
