// D96: "Cloud" is OpenRouter only, the OpenAI-compatible path is "Your own
// AI server", the Transforms line follows the live engine order, and Data
// and privacy lists the text your server cleans. Runs on the built dist:
// `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  cloudLeaves,
  cloudLiveLabel,
  cloudRouteTitle,
  providerName,
  serverCleanupOn,
  serverTextLeave,
  transformsEngineLine,
} from "../dist/lib/engine-names.js";

const order = (...routes) => routes.map(([route, enabled]) => ({ route, enabled }));

test("Cloud names OpenRouter only; the custom path is your own AI server", () => {
  assert.equal(cloudRouteTitle("openrouter"), "Cloud · OpenRouter");
  assert.equal(cloudRouteTitle("custom"), "Your own AI server");
  assert.equal(providerName("custom"), "Your own AI server (OpenAI-compatible)");
  assert.equal(providerName(undefined), "OpenRouter");
  assert.equal(cloudLiveLabel("custom", "10.0.0.5"), "Your own AI server · 10.0.0.5");
  assert.equal(cloudLiveLabel("openrouter", "llama-3.3-70b-instruct"), "Cloud · llama-3.3-70b-instruct");
  for (const p of ["custom", "openrouter"]) assert.doesNotMatch(cloudRouteTitle(p), /LiteLLM|proxy/);
  assert.doesNotMatch(cloudRouteTitle("custom"), /Cloud/);
});

test("Transforms line follows the order and skips your server", () => {
  const t = (engines, over = {}) =>
    transformsEngineLine({ engines, cloud: true, provider: "openrouter", local: true, server: false, ...over });
  assert.equal(
    t(order(["cloud", true], ["local", true], ["server", true])),
    "Uses your AI cleanup engines in their order: OpenRouter in the cloud, then this PC.",
  );
  assert.equal(
    t(order(["local", true], ["cloud", true], ["server", true])),
    "Uses your AI cleanup engines in their order: this PC, then OpenRouter in the cloud.",
  );
  // The issue: your server is the live cleaner, then the own AI server, then this PC.
  assert.equal(
    t(order(["server", true], ["cloud", true], ["local", true]), { provider: "custom", server: true }),
    "Uses your AI cleanup engines in their order: your own AI server, then this PC. Your server doesn't run transforms.",
  );
  // Cloud off: this PC only, even with its switch off (the router falls back to the loaded model).
  assert.equal(
    t(order(["cloud", false], ["local", false], ["server", false]), { cloud: false }),
    "Uses your AI cleanup engines in their order: this PC.",
  );
  // Cloud on, this PC switched off: the cloud alone.
  assert.equal(t(order(["cloud", true], ["local", false], ["server", false])), "Uses your AI cleanup engines in their order: OpenRouter in the cloud.");
  assert.equal(
    t(order(["server", true], ["local", true], ["cloud", false]), { cloud: false, local: false, server: true }),
    "Needs an AI model on this PC or OpenRouter in the cloud: turn one on under AI cleanup. Your server doesn't run transforms.",
  );
});

test("server cleanup is on only with its switch, a host and no known missing model", () => {
  const on = order(["server", true], ["local", true]);
  assert.equal(serverCleanupOn(on, "10.1.1.1", true, null), true);
  assert.equal(serverCleanupOn(on, "10.1.1.1", true, false), false);
  assert.equal(serverCleanupOn(on, "", true, true), false);
  assert.equal(serverCleanupOn(on, "10.1.1.1", false, true), false);
  assert.equal(serverCleanupOn(order(["server", false]), "10.1.1.1", true, true), false);
});

test("Data and privacy: Text to your server, and the provider rows by name", () => {
  const s = serverTextLeave(true, "10.1.1.1", true);
  assert.equal(s.title, "Text to your server");
  assert.equal(s.on, true);
  assert.match(s.desc, /10\.1\.1\.1, encrypted/);
  assert.equal(serverTextLeave(false, "", false).desc, "Off: your server doesn't clean text.");

  const base = { host: "", sttAllow: false, sttModel: "m", cleanupAllow: true, cleanupModel: "llama", zdr: true };
  const or = cloudLeaves({ ...base, provider: "openrouter" });
  assert.equal(or.text.title, "Text to the cloud");
  assert.equal(or.text.desc, "For AI cleanup with llama. Zero-retention providers only.");
  assert.equal(or.audio.desc, "Off. Turning it on asks first.");
  const own = cloudLeaves({ ...base, provider: "custom", host: "10.0.0.5", cleanupModel: "gpt-4o-mini" });
  assert.equal(own.text.title, "Text to your own AI server");
  assert.equal(own.audio.title, "Audio to your own AI server");
  assert.equal(own.text.desc, "For AI cleanup at 10.0.0.5 with gpt-4o-mini.");
  assert.match(cloudLeaves({ ...base, provider: "custom", cleanupModel: "" }).text.desc, /no model picked yet/);
});
