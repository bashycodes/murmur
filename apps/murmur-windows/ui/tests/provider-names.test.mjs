// D96: "Cloud" is OpenRouter only; the OpenAI-compatible path is "Your own
// AI server" everywhere (engine rows, Insights, Meetings, setup). Runs on
// the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  cloudCleanupUsable,
  cloudLiveLabel,
  cloudRouteTitle,
  cloudShortName,
  cloudWhere,
  ownServerOr,
  providerModel,
  providerModelLabel,
  providerName,
  providerOf,
  setupCloudCopy,
} from "../dist/lib/provider-names.js";
import { routeLabel, splitKey } from "../dist/lib/insights-text.js";

const or = (provider, over = {}) => ({
  supported: true,
  key: { set: true, masked: "sk-…abcd" },
  stt: { allow: false, model: "openai/whisper-large-v3-turbo", custom_model: "whisper-1" },
  cleanup: {
    allow: true,
    consent: true,
    provider,
    model: "meta-llama/llama-3.3-70b-instruct",
    custom: { host: "10.0.0.5", model: "vendor/gpt-4o-mini", consent: true },
  },
  ...over,
});

test("Cloud names OpenRouter only; the custom path is your own AI server", () => {
  assert.equal(cloudRouteTitle("openrouter"), "Cloud · OpenRouter");
  assert.equal(cloudRouteTitle("custom"), "Your own AI server");
  assert.equal(providerName("custom"), "Your own AI server (OpenAI-compatible)");
  assert.equal(providerName(undefined), "OpenRouter");
  assert.equal(cloudLiveLabel("custom", "10.0.0.5"), "Your own AI server · 10.0.0.5");
  assert.equal(cloudLiveLabel("openrouter", "llama"), "Cloud · llama");
  assert.equal(cloudWhere("custom"), "on your own AI server");
  assert.equal(cloudWhere("openrouter"), "in the cloud");
  assert.equal(providerOf(null), "openrouter");
  for (const p of ["custom", "openrouter"]) assert.doesNotMatch(cloudRouteTitle(p), /LiteLLM|proxy/);
});

test("a home server is never 'Cloud' or 'online' in short labels or setup", () => {
  assert.equal(cloudShortName("custom"), "Your own AI server");
  assert.equal(cloudShortName("openrouter"), "Cloud");
  const own = setupCloudCopy("custom");
  for (const s of [own.sttTitle, own.sttDesc, ...own.sttFacts, own.cleanTitle, own.cleanDesc]) {
    assert.doesNotMatch(s, /\bcloud\b|online|account key|pay per use/i, s);
  }
  assert.equal(setupCloudCopy("openrouter").cleanTitle, "Cloud");
  // Insights.
  assert.equal(routeLabel("cloud", "custom"), "Your own AI server");
  assert.equal(routeLabel("cloud"), "Cloud");
  assert.deepEqual(splitKey({ pc: 0, server: 0, cloud: 2, unknown: 0 }, "custom"), [{ route: "cloud", text: "Your own AI server 100%" }]);
});

test("one host fallback and one model pick", () => {
  assert.equal(ownServerOr(""), "your own AI server");
  assert.equal(ownServerOr(undefined), "your own AI server");
  assert.equal(ownServerOr("10.0.0.5"), "10.0.0.5");
  assert.equal(providerModel(or("openrouter"), "stt"), "openai/whisper-large-v3-turbo");
  assert.equal(providerModel(or("custom"), "stt"), "whisper-1");
  assert.equal(providerModel(or("custom"), "cleanup"), "vendor/gpt-4o-mini");
  // OpenRouter ids lose the vendor; your own server's id is shown as typed.
  assert.equal(providerModelLabel(or("openrouter"), "cleanup"), "llama-3.3-70b-instruct");
  assert.equal(providerModelLabel(or("custom"), "cleanup"), "vendor/gpt-4o-mini");
  assert.equal(providerModel(null, "cleanup"), "");
});

test("cloud cleanup is usable with consent and a key or a model", () => {
  assert.equal(cloudCleanupUsable(or("openrouter")), true);
  assert.equal(cloudCleanupUsable(or("openrouter", { key: { set: false, masked: "" } })), false);
  assert.equal(cloudCleanupUsable(or("custom", { key: { set: false, masked: "" } })), true, "own server needs no key");
  const noModel = or("custom");
  noModel.cleanup.custom.model = "";
  assert.equal(cloudCleanupUsable(noModel), false);
  const off = or("openrouter");
  off.cleanup.allow = false;
  assert.equal(cloudCleanupUsable(off), false);
  assert.equal(cloudCleanupUsable(null), false);
});
