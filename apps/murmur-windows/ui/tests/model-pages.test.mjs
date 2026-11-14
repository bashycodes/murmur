// The decluttered Speech to text and AI cleanup pages: the This PC model
// chip, the "AI engines are skipped" warning, the formatting rules that
// follow the level (D27) and the one "Give up after" for every engine.
// Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  cloudTimeout,
  llmModelChip,
  modelName,
  pcModelChip,
  pcModelEntry,
  rulesAfterLevel,
  serverCleanupLine,
  serverSpeechLine,
  skippedWarning,
  timeoutNote,
} from "../dist/lib/model-pages.js";

const DIR = "C:\\Users\\you\\AppData\\Roaming\\Murmur\\models";
const catalog = [
  { id: "pk", engine: "parakeet", dest: `${DIR}\\parakeet-folder`, installed: true, size: 661e6 },
  { id: "wb", engine: "whisper", dest: `${DIR}\\ggml-base.bin`, installed: false, size: 148e6 },
  { id: "ws", engine: "whisper", dest: `${DIR}\\ggml-small.bin`, installed: true, size: 190e6 },
  { id: "he", engine: "whisper", dest: `${DIR}\\ggml-base.bin`, language: "he", installed: true, size: 1e6 },
];

test("the model in use: the set path, else the default; never a Hebrew model; any slash or case", () => {
  assert.equal(pcModelEntry(catalog, "whisper", "", `${DIR}\\ggml-base.bin`)?.id, "wb");
  assert.equal(pcModelEntry(catalog, "whisper", `${DIR.toLowerCase().replace(/\\/g, "/")}/ggml-small.bin`, "")?.id, "ws");
  assert.equal(pcModelEntry(catalog, "parakeet", "", `${DIR}\\parakeet-folder`)?.id, "pk");
  assert.equal(pcModelEntry(catalog, "whisper", "D:\\mine.bin", `${DIR}\\ggml-base.bin`), undefined);
  assert.equal(pcModelEntry(catalog, "whisper", "", ""), undefined);
});

test("chip: an installed model reads Installed while This PC is off (status stays loading)", () => {
  assert.deepEqual(pcModelChip({ status: "loading", entry: catalog[0], canDownload: true, busy: false }), {
    text: "Installed",
    kind: "light",
    download: false,
  });
  assert.equal(pcModelChip({ status: "ready", entry: catalog[0], canDownload: true, busy: false }).text, "Ready");
});

test("chip: a model not here yet offers Download, or says so when it can't be downloaded", () => {
  assert.equal(pcModelChip({ status: "missing", entry: catalog[1], canDownload: true, busy: false }).download, true);
  const no = pcModelChip({ status: "missing", entry: catalog[1], canDownload: false, busy: false });
  assert.deepEqual([no.text, no.download], ["Not downloaded", false]);
  assert.equal(pcModelChip({ status: "loading", entry: catalog[1], canDownload: true, busy: true }).text, "Downloading…");
});

test("chip: a custom file shows only a problem", () => {
  assert.equal(pcModelChip({ status: "loading", canDownload: false, busy: false }).text, "");
  assert.equal(pcModelChip({ status: "missing", canDownload: false, busy: false }).kind, "destructive");
  assert.equal(pcModelChip({ status: "failed", canDownload: false, busy: false }).text, "Failed");
  assert.equal(pcModelChip({ status: "idle", entry: catalog[0], canDownload: true, busy: false }).text, "On standby");
});

test("cleanup chip: In use or Installed for a model here; none while downloading or not here", () => {
  assert.deepEqual(llmModelChip({ installed: true, inUse: true, busy: false }), { text: "In use", kind: "success", download: false });
  assert.deepEqual(llmModelChip({ installed: true, inUse: false, busy: false }), { text: "Installed", kind: "light", download: false });
  assert.equal(llmModelChip({ installed: true, inUse: true, busy: true }), null);
  assert.equal(llmModelChip({ installed: false, inUse: false, busy: false }), null);
});

test("warning: an AI engine on at a level without AI is skipped", () => {
  assert.equal(skippedWarning("none", "Rules only", true), "AI engines are skipped at Rules only.");
  assert.equal(skippedWarning("verbatim", "Verbatim", true), "AI engines are skipped at Verbatim.");
  assert.equal(skippedWarning("none", "Rules only", false), "");
  for (const l of ["light", "medium", "high"]) assert.equal(skippedWarning(l, l, true), "");
});

test("formatting rules follow the level (D27)", () => {
  assert.equal(rulesAfterLevel("verbatim", "light", true), false);
  assert.equal(rulesAfterLevel("none", "verbatim", false), true);
  // Regression: leaving Verbatim for an AI level left the rules off.
  assert.equal(rulesAfterLevel("light", "verbatim", false), true);
  // Between AI levels the Advanced switch is kept, off or on.
  assert.equal(rulesAfterLevel("high", "light", false), false);
  assert.equal(rulesAfterLevel("medium", "high", true), true);
  // Rules only with the rules off would read back as Verbatim.
  assert.equal(rulesAfterLevel("none", "light", false), true);
});

test("one Give up after: the cloud gets the same time within its provider's range", () => {
  assert.equal(cloudTimeout("openrouter", 3000), 3000);
  assert.equal(cloudTimeout("openrouter", 15000), 8000);
  assert.equal(cloudTimeout("custom", 15000), 15000);
  assert.equal(cloudTimeout("custom", 200), 800);
});

test("the note shows only while the cloud is on with its own, different limit", () => {
  assert.equal(timeoutNote(3000, "openrouter", null), "");
  assert.equal(timeoutNote(3000, "openrouter", 3000), "");
  // Clamped to the provider's maximum: already the same limit.
  assert.equal(timeoutNote(15000, "openrouter", 8000), "");
  assert.equal(timeoutNote(3000, "openrouter", 2500), "The cloud still waits 2.5 seconds; pick a time to use one for all.");
});

test("the Your server row names the server's cleanup model; an older server keeps the old line", () => {
  assert.equal(
    serverCleanupLine("192.0.2.10", true, "Qwen 2.5 1.5B (cleanup)"),
    "192.0.2.10 · Qwen 2.5 1.5B (cleanup). Cleans the text with its own AI model.",
  );
  assert.equal(serverCleanupLine("192.0.2.10", true, undefined), "192.0.2.10. Cleans the text with its own AI model.");
  assert.equal(serverCleanupLine("192.0.2.10", null, null), "192.0.2.10. Cleans the text with its own AI model.");
  assert.equal(serverCleanupLine("192.0.2.10", false, "Qwen"), "192.0.2.10 has no AI cleanup model: skipped.");
});

test("the Speech Your server row names the server's speech models; an older server's reads as before", () => {
  const rtt_ms = { min: 20, median: 23, max: 30 };
  const speech = { id: "parakeet-unified-en-0.6b-int8", title: "Parakeet unified 0.6B (English)" };
  const named = {
    ok: true,
    rtt_ms,
    models: { speech, languages: { he: { id: "zipformer-he", title: "Zipformer Hebrew" } } },
  };
  assert.equal(
    serverSpeechLine("homeserver", named),
    "homeserver · Parakeet unified 0.6B (English) · Hebrew: Zipformer Hebrew · 23 ms",
  );
  // A model without a title reads as its id.
  assert.equal(
    serverSpeechLine("homeserver", { ...named, models: { speech, languages: { he: { id: "zipformer-he", title: "" } } } }),
    "homeserver · Parakeet unified 0.6B (English) · Hebrew: zipformer-he · 23 ms",
  );
  // No per-language models and no round trip: just the default model.
  assert.equal(serverSpeechLine("homeserver", { ok: true, rtt_ms: null, models: { speech } }), "homeserver · Parakeet unified 0.6B (English)");
  // Cleanup-only models (no speech ones) leave the speech line as before.
  assert.equal(
    serverSpeechLine("homeserver", { ok: true, rtt_ms, models: { cleanup: { id: "qwen", title: "Qwen" } } }),
    "homeserver · 23 ms",
  );
  // An older server (no models in its hello) or one not probed yet: today's text.
  assert.equal(serverSpeechLine("homeserver", { ok: true, rtt_ms }), "homeserver · 23 ms");
  assert.equal(serverSpeechLine("homeserver", null), "homeserver");
  assert.equal(serverSpeechLine("homeserver", undefined), "homeserver");
});

test("a server's model reads as its title, else its id (like ServerModel::name)", () => {
  assert.equal(modelName({ id: "zipformer-he", title: "Zipformer Hebrew" }), "Zipformer Hebrew");
  assert.equal(modelName({ id: "zipformer-he", title: "" }), "zipformer-he");
  assert.equal(modelName({ id: "zipformer-he" }), "zipformer-he");
  assert.equal(modelName({ id: "", title: "" }), "");
  assert.equal(modelName(undefined), "");
});
