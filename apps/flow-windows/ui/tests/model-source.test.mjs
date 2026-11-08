// "Download from" (D79): the server is the default source while one is set,
// Hugging Face otherwise or when picked; a catalogue model the server lacks is
// "Not on your server"; setup's model step offers Skip when speech runs on the
// server. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { effectiveSource, modelStepFoot, onServer, sourceHint } from "../dist/lib/model-source.js";

test("the server is the default source, only while one is set", () => {
  assert.equal(effectiveSource(undefined, true), "server");
  assert.equal(effectiveSource("server", true), "server");
  assert.equal(effectiveSource("huggingface", true), "huggingface");
  assert.equal(effectiveSource("server", false), "huggingface");
  assert.equal(effectiveSource(undefined, false), "huggingface");
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

test("the hint names where the files come from", () => {
  assert.equal(sourceHint("server", true, "box.example.net"), "box.example.net · no internet needed");
  assert.equal(sourceHint("huggingface", true, "box.example.net"), "huggingface.co · needs the internet");
  assert.equal(sourceHint("huggingface", false, ""), "No server set up, so models come from huggingface.co");
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
