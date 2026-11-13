// Setup's AI cleanup step offers "Your server" (D36 server cleanup) once a
// server is saved in step 2, recommends it over this PC unless the server is
// known to lack a cleanup model, and saves the same cleanup order as
// Settings → AI cleanup's switch. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { enginesFor, initialCleanup, recommendedCleanup, serverOption } from "../dist/lib/setup-cleanup.js";
import { withEngine } from "../dist/lib/cleanup-order.js";

const D36 = [
  { route: "local", enabled: true },
  { route: "server", enabled: true },
  { route: "cloud", enabled: false },
];
const saved = { build: true, saved: true, llm: true };

test("no server saved: Your server is disabled with a reason, This PC is recommended", () => {
  const facts = { build: true, saved: false, llm: null };
  const o = serverOption(facts);
  assert.equal(o.hidden, false);
  assert.equal(o.disabled, true);
  assert.match(o.reason, /step 2/);
  assert.equal(recommendedCleanup(facts, true), "pc");
});

test("build without a server engine: Your server is hidden", () => {
  const facts = { build: false, saved: false, llm: null };
  assert.equal(serverOption(facts).hidden, true);
  assert.equal(recommendedCleanup(facts, true), "pc");
  assert.equal(recommendedCleanup(facts, false), null);
});

test("server with a cleanup model: usable and recommended over This PC", () => {
  assert.deepEqual(serverOption(saved), { hidden: false, disabled: false, reason: "" });
  assert.equal(recommendedCleanup(saved, true), "server");
});

test("server not asked yet: still recommended (only a known 'no model' demotes it)", () => {
  const facts = { ...saved, llm: null };
  assert.equal(serverOption(facts).disabled, false);
  assert.equal(recommendedCleanup(facts, true), "server");
  assert.equal(recommendedCleanup({ ...saved, llm: undefined }, true), "server");
});

test("server without a cleanup model: still pickable, says why, This PC recommended", () => {
  const facts = { ...saved, llm: false };
  const o = serverOption(facts);
  assert.equal(o.disabled, false);
  assert.match(o.reason, /no AI cleanup model/);
  assert.equal(recommendedCleanup(facts, true), "pc");
});

test("choosing Your server turns its D36 switch on and puts it first", () => {
  const off = withEngine(D36, "server", false);
  assert.deepEqual(enginesFor("server", off), [
    { route: "server", enabled: true },
    { route: "local", enabled: true },
    { route: "cloud", enabled: false },
  ]);
  // Already first and on: nothing to save.
  assert.equal(enginesFor("server", enginesFor("server", D36)), null);
});

test("choosing This PC after Your server puts this PC ahead again; otherwise the order is kept", () => {
  const serverFirst = enginesFor("server", D36);
  assert.deepEqual(enginesFor("pc", serverFirst), [
    { route: "local", enabled: true },
    { route: "server", enabled: true },
    { route: "cloud", enabled: false },
  ]);
  assert.equal(enginesFor("pc", D36), null);
  assert.equal(enginesFor("rules", D36), null);
  assert.equal(enginesFor("cloud", D36), null);
});

test("withEngine flips only that switch and does not touch the input", () => {
  const out = withEngine(D36, "server", false);
  assert.deepEqual(out.map((e) => e.enabled), [true, false, false]);
  assert.equal(D36[1].enabled, true);
});

test("selection when the step opens follows the saved settings", () => {
  const base = { cloudAllow: false, aiLevel: true, llmEnabled: false, engines: D36 };
  // Fresh install with a server saved: the server cleans (D36 default on).
  assert.equal(initialCleanup(base, true), "server");
  // No server saved: rules, as before.
  assert.equal(initialCleanup(base, false), "rules");
  // This PC on and first.
  assert.equal(initialCleanup({ ...base, llmEnabled: true }, true), "pc");
  // Server first in the order wins over this PC.
  assert.equal(initialCleanup({ ...base, llmEnabled: true, engines: enginesFor("server", D36) }, true), "server");
  // Server switched off.
  assert.equal(initialCleanup({ ...base, engines: withEngine(D36, "server", false) }, true), "rules");
  assert.equal(initialCleanup({ ...base, aiLevel: false }, true), "rules");
  assert.equal(initialCleanup({ ...base, cloudAllow: true }, true), "cloud");
});
