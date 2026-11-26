// This PC against the machine-class table (D117): the words setup and
// Settings show for `machine_fit`. The table itself is tested in murmur-core.
// Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { doesNotFit, fitText, fitWarning, machineLine, recommendedRow } from "../dist/lib/machine-fit.js";
import { initialModel } from "../dist/lib/setup-model.js";
import { recommendedCleanup, startCleanup } from "../dist/lib/setup-cleanup.js";

const GIB = 2 ** 30;
const small = {
  total_ram: 6 * GIB,
  available_ram: 2.2 * GIB,
  free_disk: 40e9,
  gpu: false,
  whisper_gpu: false,
  speech: "parakeet-tdt-0.6b-v2-int8",
  cleanup: null,
  hebrew: "zipformer-he-int8",
  fit: {
    "parakeet-tdt-0.6b-v2-int8": { fits: true, reasons: [] },
    "qwen2.5-1.5b-instruct-q4_k_m": { fits: false, reasons: ["needs 8 GB RAM"] },
    "llama-3.2-3b-instruct-q4_k_m": { fits: false, reasons: ["needs 8 GB RAM", "needs a graphics chip"] },
  },
};

test("fitText: fits, doesn't fit with the reasons, nothing when unknown", () => {
  assert.equal(fitText(small, "parakeet-tdt-0.6b-v2-int8"), "Fits this PC");
  assert.equal(fitText(small, "llama-3.2-3b-instruct-q4_k_m"), "Doesn't fit: needs 8 GB RAM, needs a graphics chip");
  assert.equal(fitText(small, "my-own-file"), "");
  assert.equal(fitText(undefined, "parakeet-tdt-0.6b-v2-int8"), "");
  assert.equal(fitText(small, null), "");
});

test("a pick that doesn't fit warns but is allowed; unknown never warns", () => {
  assert.equal(doesNotFit(small, "qwen2.5-1.5b-instruct-q4_k_m"), true);
  assert.equal(doesNotFit(small, "parakeet-tdt-0.6b-v2-int8"), false);
  assert.equal(doesNotFit(undefined, "qwen2.5-1.5b-instruct-q4_k_m"), false);
  assert.equal(
    fitWarning(small, "qwen2.5-1.5b-instruct-q4_k_m", "Qwen2.5 1.5B Instruct"),
    "Qwen2.5 1.5B Instruct may not run well on this PC (needs 8 GB RAM). You can still use it.",
  );
  assert.equal(fitWarning(small, "parakeet-tdt-0.6b-v2-int8", "English — fast"), "");
});

test("machineLine names RAM, disk and the graphics chip", () => {
  assert.equal(machineLine(small), "This PC: 6 GB RAM (2 GB free) · 40 GB free for models · no graphics chip for AI");
  assert.equal(
    machineLine({ ...small, total_ram: 0, available_ram: undefined, free_disk: undefined, gpu: true, gpu_device: "Test iGPU" }),
    "This PC: graphics chip: Test iGPU",
  );
  assert.equal(machineLine(undefined), "");
});

test("step 3 preselects the table's row on a new install, keeps the model in use on a re-run", () => {
  const ids = ["parakeet-tdt-0.6b-v2-int8", "parakeet-unified-en-0.6b-int8"];
  assert.equal(recommendedRow(small, ids), "parakeet");
  const tiny = { ...small, total_ram: 3 * GIB, speech: "whisper-base" };
  assert.equal(recommendedRow(tiny, ids), "whisper");
  assert.equal(recommendedRow(undefined, ids), "parakeet");
  assert.equal(initialModel("parakeet", false, "missing", recommendedRow(tiny, ids)), "whisper");
  assert.equal(initialModel("parakeet", true, "missing", "whisper"), "parakeet");
  assert.equal(initialModel("whisper", false, "unknown"), "parakeet");
});

test("step 4 recommends This PC only when the table picks a cleanup model for it", () => {
  const noServer = { build: true, saved: false, llm: null };
  assert.equal(recommendedCleanup(noServer, true), "pc");
  assert.equal(recommendedCleanup(noServer, true, false), null);
  // A saved server still wins.
  assert.equal(recommendedCleanup({ build: true, saved: true, llm: true }, true, false), "server");
});

test("step 4 starts on the table's pick on a fresh install, keeps a saved choice", () => {
  // Fresh install: This PC at 8 GB (or RAM not known yet), else Rules only.
  assert.equal(startCleanup("rules", false, true, true), "pc");
  assert.equal(startCleanup("rules", false, true, false), "rules");
  assert.equal(startCleanup("rules", false, false, true), "rules");
  // A server or Cloud already turned on stays.
  assert.equal(startCleanup("server", false, true, true), "server");
  assert.equal(startCleanup("cloud", false, true, false), "cloud");
  // A re-run keeps what is saved.
  assert.equal(startCleanup("rules", true, true, true), "rules");
  assert.equal(startCleanup("pc", true, true, false), "pc");
});
