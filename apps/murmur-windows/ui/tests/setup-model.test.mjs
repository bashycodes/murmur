// Setup's speech-model step (UX review on 1.3.0): only for This PC, one
// recommended row, a state chip per row, and a Whisper row named after the
// file it points at. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { chip, fileState, initialModel, PARAKEET_TITLE, setupSteps, whisperName, whisperRow } from "../dist/lib/setup-model.js";

test("the model step is there only when this PC does the listening", () => {
  assert.deepEqual(setupSteps(true, "pc"), ["welcome", "engine", "model", "cleanup", "try"]);
  assert.deepEqual(setupSteps(true, "server"), ["welcome", "engine", "cleanup", "try"]);
  assert.deepEqual(setupSteps(true, "cloud"), ["welcome", "engine", "cleanup", "try"]);
  assert.deepEqual(setupSteps(false, "pc"), ["welcome", "engine", "cleanup", "try"]);
});

test("chip: a download wins, then the file; unknown shows none", () => {
  assert.deepEqual(chip("missing", 34), { state: "downloading", text: "Downloading 34%", kind: "light" });
  assert.deepEqual(chip("present", 0), { state: "downloading", text: "Downloading 0%", kind: "light" });
  assert.deepEqual(chip("present", null), { state: "installed", text: "Installed", kind: "success" });
  assert.deepEqual(chip("missing", null), { state: "missing", text: "Not installed", kind: "warning" });
  // Nothing says the file is there: never "Installed" on a guess.
  assert.equal(chip("unknown", null), null);
});

test("fileState: only missing says no; a never-loaded slot with no path says nothing", () => {
  assert.equal(fileState(null), "unknown");
  assert.equal(fileState({ state: "missing", path: "C:\\m\\a.bin" }), "missing");
  assert.equal(fileState({ state: "idle", path: "" }), "unknown");
  assert.equal(fileState({ state: "loading", path: "" }), "unknown");
  assert.equal(fileState({ state: "idle", path: "C:\\m\\a.bin" }), "present");
  assert.equal(fileState({ state: "ready", path: "C:\\m\\a.bin" }), "present");
  assert.equal(fileState({ state: "failed", path: "C:\\m\\a.bin" }), "present");
});

test("initialModel: a re-run or a file that is there keeps the model in use", () => {
  assert.equal(initialModel("whisper", true, "unknown"), "whisper");
  assert.equal(initialModel("whisper", false, "present"), "whisper");
  // A new install's built-in Whisper default was never chosen.
  assert.equal(initialModel("whisper", false, "missing"), "parakeet");
  assert.equal(initialModel("whisper", false, "unknown"), "parakeet");
});

test("the recommended row's name", () => {
  assert.equal(PARAKEET_TITLE, "English — fast");
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
