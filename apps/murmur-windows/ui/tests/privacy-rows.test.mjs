// D96: Data and privacy lists the text your server cleans, and names the
// provider rows by the saved provider. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { serverCleanupOn } from "../dist/lib/cleanup-order.js";
import { cloudLeaves, serverTextLeave } from "../dist/lib/privacy-rows.js";

const order = (...routes) => routes.map(([route, enabled]) => ({ route, enabled }));

test("server cleanup is on only with its switch, a host and no known missing model", () => {
  const on = order(["server", true], ["local", true]);
  assert.equal(serverCleanupOn(on, "10.1.1.1", true, null), true);
  assert.equal(serverCleanupOn(on, "10.1.1.1", true, false), false);
  assert.equal(serverCleanupOn(on, "", true, true), false);
  assert.equal(serverCleanupOn(on, "10.1.1.1", false, true), false);
  assert.equal(serverCleanupOn(order(["server", false]), "10.1.1.1", true, true), false);
});

test("Text to your server", () => {
  const s = serverTextLeave(true, "10.1.1.1", true);
  assert.equal(s.title, "Text to your server");
  assert.equal(s.on, true);
  assert.match(s.desc, /10\.1\.1\.1, encrypted/);
  assert.equal(serverTextLeave(false, "", false).desc, "Off: your server doesn't clean text.");
});

test("the provider rows by name", () => {
  const view = (provider, host = "10.0.0.5") => ({
    privacy: { zdr: true },
    stt: { allow: false, model: "openai/whisper", custom_model: "" },
    cleanup: { allow: true, provider, model: "meta/llama", custom: { host, model: "gpt-4o-mini" } },
  });
  const or = cloudLeaves(view("openrouter"));
  assert.equal(or.text.title, "Text to the cloud");
  assert.equal(or.text.desc, "For AI cleanup with meta/llama. Zero-retention providers only.");
  assert.equal(or.audio.desc, "Off. Turning it on asks first.");
  const own = cloudLeaves(view("custom"));
  assert.equal(own.text.title, "Text to your own AI server");
  assert.equal(own.audio.title, "Audio to your own AI server");
  assert.equal(own.text.desc, "For AI cleanup at 10.0.0.5 with gpt-4o-mini.");
  assert.match(cloudLeaves(view("custom", "")).text.desc, /at your own AI server with/);
});
