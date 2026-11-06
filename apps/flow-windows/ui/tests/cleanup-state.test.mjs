// Regression (Windows 1.1.2): with a key saved and consent given but the
// cloud engine off in Cleanup order, the cloud server card's "Try it" row
// showed "cloud_cleanup_not_configured — … save a key". It now says the
// engine is off in Cleanup order and offers to turn it on. Runs on the
// built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanupBlockedLine, cleanupTestFailText } from "../dist/lib/cleanup-state.js";

const status = (over = {}) => ({
  supported: true,
  enabled: true,
  consent: true,
  blocked: null,
  circuit: null,
  model: "vendor/some-model",
  timeout_ms: 2500,
  last: null,
  month_cost_usd: 0,
  provider: "openrouter",
  ...over,
});
const view = ({ keySet = true, baseUrl = "", model = "" } = {}) => ({
  key: { set: keySet, masked: keySet ? "sk-…abcd" : "" },
  cleanup: { custom: { base_url: baseUrl, model, host: "", key: { set: false, masked: "" } } },
});

test("off in Cleanup order with a key saved: says it is off, offers to turn it on", () => {
  // The router reports the D28 code whenever cleanup is not wanted.
  const l = cleanupBlockedLine(status({ enabled: false, blocked: "cloud_cleanup_not_configured" }), view());
  assert.ok(l);
  assert.match(l.text, /off in Cleanup order/);
  assert.doesNotMatch(l.text, /cloud_cleanup_not_configured|key/);
  assert.equal(l.turnOn, true);
});

test("on in Cleanup order but no key: asks for a key, no turn-on offer", () => {
  const l = cleanupBlockedLine(status({ blocked: "cloud_cleanup_not_configured" }), view({ keySet: false }));
  assert.ok(l);
  assert.match(l.text, /save an OpenRouter key/);
  assert.doesNotMatch(l.text, /cloud_cleanup_not_configured/);
  assert.equal(l.turnOn, undefined);
});

test("on but no consent: asks for the consent", () => {
  const l = cleanupBlockedLine(status({ consent: false, blocked: "cloud_cleanup_not_configured" }), view());
  assert.match(l.text, /consent/);
});

test("custom server without an address: asks for it, not for a key", () => {
  const l = cleanupBlockedLine(
    status({ provider: "custom", blocked: "cloud_cleanup_not_configured" }),
    view({ keySet: false }),
  );
  assert.match(l.text, /server address/);
});

test("ready: nothing blocks it", () => {
  assert.equal(cleanupBlockedLine(status(), view()), null);
});

test("other blocks are worded, never the raw code", () => {
  const known = cleanupBlockedLine(status({ blocked: "budget_exceeded" }), view());
  assert.equal(known.text, "Not used: monthly budget reached.");
  const unknown = cleanupBlockedLine(status({ blocked: "some_new_code" }), view());
  assert.doesNotMatch(unknown.text, /some_new_code/);
});

test("a failed Test names the cause in words", () => {
  assert.equal(cleanupTestFailText("auth_failed"), "Failed: key rejected");
  assert.equal(
    cleanupTestFailText("cloud_not_configured", "save an OpenRouter key first"),
    "Failed: save an OpenRouter key first",
  );
  assert.doesNotMatch(cleanupTestFailText("weird_code", "HTTP 418"), /weird_code/);
});
