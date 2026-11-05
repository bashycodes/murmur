// Regression: with "Custom server" picked but not saved yet (address or
// model still empty), the key's Save reloaded the saved settings and the
// selector flipped back to OpenRouter (Windows 1.1.2). Runs on the built
// dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ProviderChoice } from "../dist/lib/cleanup-provider.js";

test("an unsaved custom pick survives a reload of the saved settings", () => {
  const c = new ProviderChoice();
  c.pick("custom", "openrouter");
  // Key saved -> loadOr -> showOr with the stored provider still openrouter.
  assert.equal(c.shown("openrouter"), "custom");
});

test("once saved, the stored provider shows", () => {
  const c = new ProviderChoice();
  c.pick("custom", "openrouter");
  c.settle();
  assert.equal(c.shown("custom"), "custom");
  assert.equal(c.shown("openrouter"), "openrouter");
});

test("refused consent drops the pick", () => {
  const c = new ProviderChoice();
  c.pick("openrouter", "custom");
  c.settle();
  assert.equal(c.shown("custom"), "custom");
});

test("picking the stored provider again leaves no pending pick", () => {
  const c = new ProviderChoice();
  c.pick("custom", "openrouter");
  c.pick("openrouter", "openrouter");
  assert.equal(c.shown("custom"), "custom");
});

test("a reload waits for a provider save that is still running", async () => {
  const c = new ProviderChoice();
  let stored = "openrouter";
  let release;
  const gate = new Promise((r) => (release = r));
  // The model field's change (fired by the click on Save) starts the save...
  void c.run(async () => {
    await gate;
    stored = "custom";
  });
  // ...and the key's Save reloads only after it.
  const reloaded = c.idle().then(() => stored);
  release();
  assert.equal(await reloaded, "custom");
});

test("a failed save does not block later ones", async () => {
  const c = new ProviderChoice();
  await assert.rejects(c.run(async () => { throw new Error("bad url"); }));
  assert.equal(await c.run(async () => 1), 1);
  await c.idle();
});

test("the card keeps typed fields only while a pick is unsaved", () => {
  const c = new ProviderChoice();
  assert.equal(c.picking, false);
  c.pick("custom", "openrouter");
  assert.equal(c.picking, true);
  c.settle();
  assert.equal(c.picking, false);
});
