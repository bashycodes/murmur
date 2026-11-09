// "Download from" (D79): setup's switch over the backend's source (steps 3
// and 4), with step 2's server fields when Your server is picked with none
// saved; a catalogue model the server lacks is "Not on your server"; the
// model step offers Skip when speech runs on the server. Runs on the built
// dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { modelStepFoot, onServer, sourceHint, sourceView } from "../dist/lib/model-source.js";

test("the switch follows the backend until the user picks", () => {
  // A server is saved and the backend downloads from it (the default).
  const v = sourceView({ pick: null, effective: "server", remote: true, serverSet: true });
  assert.deepEqual(v, { shown: "server", serverButton: true, serverForm: false, downloads: true, hfLinks: false });
  // No server: the backend uses Hugging Face, the links to its pages show.
  const hf = sourceView({ pick: null, effective: "huggingface", remote: true, serverSet: false });
  assert.deepEqual(hf, { shown: "huggingface", serverButton: true, serverForm: false, downloads: true, hfLinks: true });
});

test("air-gapped setup on This PC: Your server with none saved shows the server fields, no Download until it's saved", () => {
  const v = sourceView({ pick: "server", effective: "huggingface", remote: true, serverSet: false });
  assert.equal(v.shown, "server");
  assert.equal(v.serverForm, true);
  // Download would still fetch from Hugging Face, so it waits for the server.
  assert.equal(v.downloads, false);
  assert.equal(v.hfLinks, false);
  assert.equal(sourceHint(v, ""), "Add your server below. No internet needed.");
  // Connected: the backend now downloads from the server; the fields go.
  const saved = sourceView({ pick: "server", effective: "server", remote: true, serverSet: true });
  assert.deepEqual(saved, { shown: "server", serverButton: true, serverForm: false, downloads: true, hfLinks: false });
  assert.equal(sourceHint(saved, "box.example.net"), "box.example.net · no internet needed");
});

test("picking Hugging Face waits for the backend before offering Download", () => {
  const v = sourceView({ pick: "huggingface", effective: "server", remote: true, serverSet: true });
  assert.equal(v.shown, "huggingface");
  assert.equal(v.downloads, false);
  assert.equal(sourceHint(v, "box.example.net"), "huggingface.co · needs the internet");
});

test("a build without servers offers Hugging Face only", () => {
  const v = sourceView({ pick: "server", effective: "huggingface", remote: false, serverSet: false });
  assert.deepEqual(v, { shown: "huggingface", serverButton: false, serverForm: false, downloads: true, hfLinks: true });
});

test("a catalogue model is on the server only as a usable tagged copy", () => {
  const lib = {
    models: [
      { catalogue_id: "parakeet-tdt-0.6b-v2-int8", slot: "stt_parakeet" },
      // Tagged, but its pins differ: listed with a reason, no slot.
      { catalogue_id: "whisper-small" },
      { slot: "stt_whisper" },
    ],
  };
  assert.equal(onServer("parakeet-tdt-0.6b-v2-int8", lib), true);
  assert.equal(onServer("whisper-small", lib), false);
  assert.equal(onServer("whisper-base", lib), false);
  // Not read yet, or unreadable: unknown, so Download stays offered.
  assert.equal(onServer("whisper-base", undefined), undefined);
  assert.equal(onServer("whisper-base", { error: "refused", models: [] }), undefined);
});

test("setup's model step: skip while speech runs on the server and nothing is ready", () => {
  const base = { needed: false, serverEngine: true, ready: false, busy: false };
  assert.deepEqual(modelStepFoot(base), { skip: true, next: "hidden" });
  assert.deepEqual(modelStepFoot({ ...base, busy: true }), { skip: true, next: "enabled" });
  assert.deepEqual(modelStepFoot({ ...base, ready: true }), { skip: false, next: "enabled" });
  // This PC does the listening: a model is required, no skip.
  const pc = { needed: true, serverEngine: false, ready: false, busy: false };
  assert.deepEqual(modelStepFoot(pc), { skip: false, next: "disabled" });
  assert.deepEqual(modelStepFoot({ ...pc, busy: true }), { skip: false, next: "enabled" });
  // OpenRouter does the listening: optional, Continue as before.
  assert.deepEqual(modelStepFoot({ needed: false, serverEngine: false, ready: false, busy: false }), { skip: false, next: "enabled" });
});
