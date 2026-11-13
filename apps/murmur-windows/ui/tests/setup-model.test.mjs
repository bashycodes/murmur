// Setup's speech-model step (UX review on 1.3.0): only for This PC, one
// recommended row, a state chip per row, and a Whisper row named after the
// file it points at. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { chip, filePresent, initialModel, setupSteps, whisperName, whisperRow } from "../dist/lib/setup-model.js";

test("the model step is there only when this PC does the listening", () => {
  assert.deepEqual(setupSteps(true, "pc"), ["welcome", "engine", "model", "cleanup", "try"]);
  assert.deepEqual(setupSteps(true, "server"), ["welcome", "engine", "cleanup", "try"]);
  assert.deepEqual(setupSteps(true, "cloud"), ["welcome", "engine", "cleanup", "try"]);
  assert.deepEqual(setupSteps(false, "pc"), ["welcome", "engine", "cleanup", "try"]);
});

test("chip: a download wins, then the file being there", () => {
  assert.deepEqual(chip(false, 34), { state: "downloading", text: "Downloading 34%", kind: "light" });
  assert.deepEqual(chip(true, 0), { state: "downloading", text: "Downloading 0%", kind: "light" });
  assert.deepEqual(chip(true, null), { state: "installed", text: "Installed", kind: "success" });
  assert.deepEqual(chip(false, null), { state: "missing", text: "Not installed", kind: "warning" });
});

test("filePresent: only missing says no; a never-loaded slot with no path says nothing", () => {
  assert.equal(filePresent(null), undefined);
  assert.equal(filePresent({ state: "missing", path: "C:\\m\\a.bin" }), false);
  assert.equal(filePresent({ state: "idle", path: "" }), undefined);
  assert.equal(filePresent({ state: "loading", path: "" }), undefined);
  assert.equal(filePresent({ state: "idle", path: "C:\\m\\a.bin" }), true);
  assert.equal(filePresent({ state: "ready", path: "C:\\m\\a.bin" }), true);
  assert.equal(filePresent({ state: "failed", path: "C:\\m\\a.bin" }), true);
});

test("initialModel: a re-run or a file that is there keeps the model in use", () => {
  assert.equal(initialModel("whisper", true, undefined), "whisper");
  assert.equal(initialModel("whisper", false, true), "whisper");
  // A new install's built-in Whisper default was never chosen.
  assert.equal(initialModel("whisper", false, false), "parakeet");
  assert.equal(initialModel("whisper", false, undefined), "parakeet");
});

const catalog = [
  { id: "whisper-base", title: "Whisper base", engine: "whisper", dest: "C:/Murmur/models/ggml-base.bin" },
  { id: "whisper-small-q5_1", title: "Whisper small", engine: "whisper", dest: "C:/Murmur/models/ggml-small-q5_1.bin" },
  { id: "whisper-he", title: "Whisper Hebrew", engine: "whisper", dest: "C:/Murmur/models/ggml-he.bin", language: "he" },
  { id: "parakeet-tdt-0.6b-v2-int8", title: "Parakeet", engine: "parakeet", dest: "C:/Murmur/models/parakeet" },
];

test("whisperName: the file's name without ggml- and .bin", () => {
  assert.equal(whisperName("C:\\Users\\u\\models\\ggml-large-v3-turbo.bin"), "Whisper large-v3-turbo");
  assert.equal(whisperName("/m/ggml-base.en.bin"), "Whisper base.en");
  assert.equal(whisperName(""), "Whisper");
});

test("whisperRow: the label always matches the file", () => {
  assert.deepEqual(whisperRow(null, catalog), { id: "whisper-base", title: "Whisper base" });
  assert.deepEqual(whisperRow("  ", catalog), { id: "whisper-base", title: "Whisper base" });
  // A catalogue file, case and slashes aside.
  assert.deepEqual(whisperRow("c:\\murmur\\MODELS\\ggml-small-q5_1.bin", catalog), { id: "whisper-small-q5_1", title: "Whisper small" });
  // A file of the user's own: named after it, nothing to download.
  assert.deepEqual(whisperRow("D:\\x\\ggml-large-v3-turbo.bin", catalog), { id: null, title: "Whisper large-v3-turbo" });
  // A language model is never the speech row.
  assert.deepEqual(whisperRow("C:/Murmur/models/ggml-he.bin", catalog), { id: null, title: "Whisper he" });
  // No catalogue (an old backend): still a name.
  assert.deepEqual(whisperRow(null, []), { id: null, title: "Whisper base" });
});
