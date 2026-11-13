// D96: the Transforms banner follows the router. Each case below is one
// of murmur-router's `command_cleaner_follows_the_cleanup_route`
// (crates/murmur-router/tests/cleanup_policy.rs) or `legacy_plan` branches
// (router.rs): if the router changes, these must change with it. Runs on
// the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { transformEngines, transformsEngineLine } from "../dist/lib/transforms-line.js";

const order = (...routes) => routes.map(([route, enabled]) => ({ route, enabled }));
const t = (engines, over = {}) => ({ engines, cloudAllowed: true, provider: "openrouter", local: true, server: false, ...over });

test("pinned to the router's command_cleaner cases", () => {
  // Cloud first with consent + key: the cloud gate, this PC as its fallback.
  assert.deepEqual(transformEngines(t(order(["cloud", true], ["local", true], ["server", true]))), ["cloud", "local"]);
  // Cloud cleanup not allowed: the on-device LLM only, never the cloud.
  assert.deepEqual(transformEngines(t(order(["cloud", true], ["local", true]), { cloudAllowed: false })), ["local"]);
  // ...and no LLM at all: nothing.
  assert.deepEqual(transformEngines(t(order(["cloud", true], ["local", true]), { cloudAllowed: false, local: false })), []);
  // Local first: the loaded LLM wins (alone, no cloud fallback); without one, the cloud.
  assert.deepEqual(transformEngines(t(order(["local", true], ["cloud", true]))), ["local"]);
  assert.deepEqual(transformEngines(t(order(["local", true], ["cloud", true]), { local: false })), ["cloud"]);
});

test("wanted = allowed and listed, ready or not; the server never runs transforms", () => {
  // The cloud is wanted even when it can't answer yet (no key): it is still tried first.
  assert.deepEqual(transformEngines(t(order(["server", true], ["cloud", true], ["local", true]))), ["cloud", "local"]);
  // Cloud first but this PC switched off: the cloud alone.
  assert.deepEqual(transformEngines(t(order(["cloud", true], ["local", false]))), ["cloud"]);
  // Cloud switched off in the order: not wanted, so this PC answers even with its switch off.
  assert.deepEqual(transformEngines(t(order(["cloud", false], ["local", false]))), ["local"]);
  // Only the server on: nothing runs transforms.
  assert.deepEqual(transformEngines(t(order(["server", true], ["cloud", false], ["local", false]), { local: false, server: true })), []);
});

test("the banner sentence", () => {
  assert.equal(
    transformsEngineLine(t(order(["cloud", true], ["local", true]))),
    "Uses OpenRouter in the cloud, then this PC.",
  );
  // The issue: your server is the live cleaner, then the own AI server, then this PC.
  assert.equal(
    transformsEngineLine(t(order(["server", true], ["cloud", true], ["local", true]), { provider: "custom", server: true })),
    "Uses your own AI server, then this PC. Your server doesn't run transforms.",
  );
  assert.equal(
    transformsEngineLine(t(order(["server", true], ["local", true], ["cloud", false]), { cloudAllowed: false, local: false, server: true })),
    "Needs an AI model on this PC or OpenRouter in the cloud: turn one on under AI cleanup. Your server doesn't run transforms.",
  );
});
