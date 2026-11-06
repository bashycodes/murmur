// Regression (Windows 1.1.2): with a key saved and consent given but the
// cloud engine off in Cleanup order, the cloud server card's "Try it" row
// showed "cloud_cleanup_not_configured — … save a key". It now says the
// engine is off in Cleanup order and offers to turn it on. Runs on the
// built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanupBlockedLine, cleanupEngineName, cleanupTestFailText } from "../dist/lib/cleanup-state.js";

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

test("custom server without an address: asks for it, not for consent or a key", () => {
  // The backend reports no consent while there is no host to consent to.
  const l = cleanupBlockedLine(
    status({ provider: "custom", host: "", consent: false, blocked: "cloud_cleanup_not_configured" }),
    view({ keySet: false }),
  );
  assert.equal(l.text, "Not used: enter the server address.");
});

test("custom server without a model: asks for one", () => {
  const l = cleanupBlockedLine(
    status({ provider: "custom", host: "10.0.0.5", blocked: "cloud_cleanup_not_configured" }),
    view({ baseUrl: "http://10.0.0.5:4000/v1" }),
  );
  assert.equal(l.text, "Not used: pick a model.");
});

test("the Cleanup order switch wins over any other block", () => {
  for (const blocked of ["cloud_cleanup_not_configured", "budget_exceeded", "circuit_open"]) {
    const l = cleanupBlockedLine(status({ enabled: false, consent: false, blocked }), view({ keySet: false }));
    assert.match(l.text, /off in Cleanup order/, blocked);
    assert.equal(l.turnOn, true);
  }
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
  assert.equal(cleanupTestFailText("auth_failed", null, "openrouter"), "Failed: key rejected");
  assert.equal(
    cleanupTestFailText("cloud_not_configured", "save an OpenRouter key first", "openrouter"),
    "Failed: save an OpenRouter key first",
  );
  assert.doesNotMatch(cleanupTestFailText("weird_code", null, "openrouter"), /weird_code/);
  // A sentence from the backend is kept; one carrying a code is not.
  assert.equal(cleanupTestFailText("provider_error", "HTTP 418", "custom"), "Failed: the provider returned an error (HTTP 418)");
  assert.doesNotMatch(cleanupTestFailText("provider_error", "upstream said model_not_found", "custom"), /model_not_found/);
  // The call itself failed: no error text from the page either.
  assert.equal(cleanupTestFailText(null, null, "openrouter"), "Failed: something went wrong");
});

test("no OpenRouter key hint on the custom server", () => {
  assert.doesNotMatch(cleanupTestFailText("cloud_not_configured", null, "custom"), /OpenRouter/);
});

test("the last engine is named in words", () => {
  assert.equal(cleanupEngineName("cloud", "custom", "10.0.0.5"), "10.0.0.5");
  assert.equal(cleanupEngineName("cloud", "openrouter"), "OpenRouter");
  assert.equal(cleanupEngineName("local", "openrouter"), "this PC");
  assert.equal(cleanupEngineName("server", "openrouter"), "your server");
  assert.equal(cleanupEngineName("some_new_engine", "openrouter"), "another engine");
});
