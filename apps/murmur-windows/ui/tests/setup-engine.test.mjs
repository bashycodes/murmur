// Setup's speech step recommends and pre-selects "Your server" (D73): owner
// walkthrough on a clean install. This PC stays one click away, and an engine
// already in use stays picked. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { initialEngine, recommendedEngine } from "../dist/lib/setup-engine.js";

const build = (...on) => (e) => on.includes(e);
const full = build("pc", "server", "cloud");

test("full build: Your server is recommended", () => {
  assert.equal(recommendedEngine(full), "server");
});

test("build without a server engine: This PC is recommended", () => {
  assert.equal(recommendedEngine(build("pc", "cloud")), "pc");
  assert.equal(recommendedEngine(build("cloud")), null);
});

test("clean install (this PC on by default, never chosen): Your server is pre-selected", () => {
  assert.equal(initialEngine("pc", false, full), "server");
  assert.equal(initialEngine(null, false, full), "server");
});

test("a server or OpenRouter already in use stays picked", () => {
  assert.equal(initialEngine("server", false, full), "server");
  assert.equal(initialEngine("cloud", true, full), "cloud");
});

test("setup re-run after choosing This PC keeps This PC", () => {
  assert.equal(initialEngine("pc", true, full), "pc");
});

test("builds without a server engine fall back to This PC, then OpenRouter", () => {
  assert.equal(initialEngine("pc", false, build("pc", "cloud")), "pc");
  assert.equal(initialEngine(null, false, build("cloud")), "cloud");
});
