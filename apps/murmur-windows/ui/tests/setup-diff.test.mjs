// Re-running setup (D95): what the saved settings say for each step, and the
// line before Finish that says what Finish changes. Runs on the built dist:
// `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { changesLine, choiceLabel, savedEngine, setupChanges, stepOf } from "../dist/lib/setup-diff.js";

const cfg = { engine: "server", model: "English — fast", cleanup: "server", ptt: "Right Alt", mic: null };

test("nothing changed: no changes, and the line says so", () => {
  assert.deepEqual(setupChanges(cfg, { ...cfg }), []);
  assert.equal(changesLine([]), "Nothing changes. Finish keeps your setup as it was.");
});

test("one change, in setup's words", () => {
  const ch = setupChanges(cfg, { ...cfg, cleanup: "rules" });
  assert.deepEqual(ch, [{ key: "cleanup", what: "Cleanup", from: "Your server", to: "Rules only" }]);
  assert.equal(changesLine(ch), "Finish changes Cleanup: Your server → Rules only.");
});

test("several changes, in step order", () => {
  const ch = setupChanges(cfg, { engine: "pc", model: "Whisper base", cleanup: "pc", ptt: "Ctrl+Win", mic: "USB Mic" });
  assert.deepEqual(
    ch.map((c) => c.key),
    ["engine", "model", "cleanup", "ptt", "mic"],
  );
  assert.equal(
    changesLine(ch),
    "Finish changes Speech: Your server → This PC; Speech model: English — fast → Whisper base; Cleanup: Your server → On this PC; Shortcut: Right Alt → Ctrl + Win; Microphone: System default → USB Mic.",
  );
});

test("labels for the empty cases", () => {
  const none = { engine: null, model: null, cleanup: "rules", ptt: "Ctrl+Shift+Space", mic: "" };
  assert.equal(choiceLabel("engine", none), "Not set up");
  assert.equal(choiceLabel("model", none), "None");
  assert.equal(choiceLabel("ptt", none), "Ctrl + Shift + Space");
  // An empty microphone is the system default, the same as null: no change.
  assert.equal(choiceLabel("mic", none), "System default");
  assert.deepEqual(setupChanges({ ...cfg, mic: null }, { ...cfg, mic: "" }), []);
  assert.equal(choiceLabel("cleanup", { ...cfg, cleanup: "cloud" }), "Cloud");
});

test("Change goes to the step of the choice", () => {
  assert.equal(stepOf("engine"), "engine");
  assert.equal(stepOf("model"), "model");
  assert.equal(stepOf("cleanup"), "cleanup");
  assert.equal(stepOf("ptt"), "try");
  assert.equal(stepOf("mic"), "try");
});

const routes = { route_order: ["server", "client", "cloud"], allow_client: true, allow_server: true, host: "100.64.0.10", cloudStt: false };

test("savedEngine: the first route in the order that would be used", () => {
  assert.equal(savedEngine(routes), "server");
  // A server without an address is skipped.
  assert.equal(savedEngine({ ...routes, host: "  " }), "pc");
  assert.equal(savedEngine({ ...routes, allow_server: false }), "pc");
  // Cloud first counts only when cloud speech is on.
  assert.equal(savedEngine({ ...routes, route_order: ["cloud", "server", "client"] }), "server");
  assert.equal(savedEngine({ ...routes, route_order: ["cloud", "server", "client"], cloudStt: true }), "cloud");
  assert.equal(savedEngine({ ...routes, allow_client: false, allow_server: false }), null);
});
