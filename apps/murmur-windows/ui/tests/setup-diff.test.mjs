// Re-running setup (D95): what the saved settings say for each choice, and
// the line before Finish that says what Finish changes. Choices compare as
// saved, never by display name. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { changesLine, choiceLabel, savedEngine, setupChanges, STEP_OF } from "../dist/lib/setup-diff.js";

const turbo = { asr_engine: "whisper", model_path: "C:\\m\\ggml-large-v3-turbo.bin", parakeet_model_dir: null };
const cfg = {
  engine: "server",
  host: "100.64.0.10",
  tokenSaves: 0,
  model: turbo,
  source: "server",
  cleanup: "pc",
  llmPath: "C:\\m\\qwen.gguf",
  ptt: "Right Alt",
  mic: null,
};
const L = { modelName: (m) => (m.asr_engine === "parakeet" ? "English — fast" : `W(${m.model_path})`) };

test("nothing changed: no changes, and the line says so", () => {
  assert.deepEqual(setupChanges(cfg, { ...cfg }, L), []);
  assert.equal(changesLine([]), "Nothing changes. Finish keeps your setup as it was.");
});

test("one change, in setup's words", () => {
  const ch = setupChanges({ ...cfg, cleanup: "server" }, { ...cfg, cleanup: "rules" }, L);
  assert.deepEqual(ch, [{ key: "cleanup", what: "Cleanup", from: "Your server", to: "Rules only" }]);
  assert.equal(changesLine(ch), "Finish changes Cleanup: Your server → Rules only.");
});

test("the server's address, edited while the engine stays Your server", () => {
  const ch = setupChanges(cfg, { ...cfg, host: "100.64.0.20" }, L);
  assert.equal(changesLine(ch), "Finish changes Server address: 100.64.0.10 → 100.64.0.20.");
  assert.equal(STEP_OF[ch[0].key], "engine");
  // Spaces around it are not a change (the address is saved trimmed).
  assert.deepEqual(setupChanges(cfg, { ...cfg, host: " 100.64.0.10 " }, L), []);
});

test("a new token: named, its value never shown", () => {
  const ch = setupChanges(cfg, { ...cfg, tokenSaves: 1 }, L);
  assert.deepEqual(ch.map((c) => c.key), ["token"]);
  assert.equal(changesLine(ch), "Finish changes Server token replaced.");
});

test("the cleanup model file, changed while cleanup stays On this PC", () => {
  const ch = setupChanges(cfg, { ...cfg, llmPath: "D:\\other\\llama.gguf" }, L);
  assert.equal(changesLine(ch), "Finish changes Cleanup model: qwen.gguf → llama.gguf.");
  assert.equal(STEP_OF.llmPath, "cleanup");
  assert.equal(choiceLabel("llmPath", { ...cfg, llmPath: null }, L), "None");
});

test("download source", () => {
  const ch = setupChanges(cfg, { ...cfg, source: "huggingface" }, L);
  assert.equal(changesLine(ch), "Finish changes Download from: Your server → Hugging Face.");
  assert.equal(STEP_OF.source, "cleanup");
});

test("the speech model compares as saved, not by its name", () => {
  // The same file named two ways (the catalogue was missing, then loaded): no change.
  let n = 0;
  const flaky = { modelName: () => `name ${n++}` };
  assert.deepEqual(setupChanges(cfg, { ...cfg, model: { ...turbo } }, flaky), []);
  // Another file under the same name: a change.
  const same = { modelName: () => "Whisper" };
  assert.equal(setupChanges(cfg, { ...cfg, model: { ...turbo, model_path: "C:\\m\\ggml-base.bin" } }, same).length, 1);
  // Parakeet keeps a Whisper path set: only its own folder counts.
  const pk = { asr_engine: "parakeet", model_path: "a.bin", parakeet_model_dir: "C:\\pk" };
  assert.deepEqual(setupChanges({ ...cfg, model: pk }, { ...cfg, model: { ...pk, model_path: "b.bin" } }, L), []);
  assert.equal(changesLine(setupChanges(cfg, { ...cfg, model: pk }, L)), `Finish changes Speech model: W(${turbo.model_path}) → English — fast.`);
});

test("several changes, in step order", () => {
  const ch = setupChanges(cfg, { ...cfg, engine: "pc", host: "", cleanup: "rules", ptt: "Ctrl+Win", mic: "USB Mic" }, L);
  assert.deepEqual(
    ch.map((c) => c.key),
    ["engine", "host", "cleanup", "ptt", "mic"],
  );
  assert.equal(
    changesLine(ch),
    "Finish changes Speech: Your server → This PC; Server address: 100.64.0.10 → None; Cleanup: On this PC → Rules only; Shortcut: Right Alt → Ctrl + Win; Microphone: System default → USB Mic.",
  );
});

test("labels for the empty cases", () => {
  const none = { ...cfg, engine: null, model: null, ptt: "Ctrl+Shift+Space", mic: "" };
  assert.equal(choiceLabel("engine", none, L), "Not set up");
  assert.equal(choiceLabel("model", none, L), "None");
  assert.equal(choiceLabel("ptt", none, L), "Ctrl + Shift + Space");
  // An empty microphone is the system default, the same as null: no change.
  assert.equal(choiceLabel("mic", none, L), "System default");
  assert.deepEqual(setupChanges({ ...cfg, mic: null }, { ...cfg, mic: "" }, L), []);
  assert.equal(choiceLabel("cleanup", { ...cfg, cleanup: "cloud" }, L), "Cloud");
});

test("D96: the line names your own AI server, not Cloud, when that is the provider", () => {
  const own = { ...L, provider: "custom" };
  assert.equal(choiceLabel("cleanup", { ...cfg, cleanup: "cloud" }, own), "Your own AI server");
  assert.equal(choiceLabel("engine", { ...cfg, engine: "cloud" }, own), "Your own AI server");
  assert.equal(choiceLabel("engine", { ...cfg, engine: "cloud" }, { ...L, provider: "openrouter" }), "Cloud");
  assert.equal(
    changesLine(setupChanges({ ...cfg, cleanup: "cloud" }, { ...cfg, cleanup: "rules" }, own)),
    "Finish changes Cleanup: Your own AI server → Rules only.",
  );
});

test("Change goes to the step of the choice", () => {
  assert.deepEqual(STEP_OF, {
    engine: "engine",
    host: "engine",
    token: "engine",
    model: "model",
    hebrew: "model",
    lid: "model",
    language: "model",
    source: "cleanup",
    cleanup: "cleanup",
    llmPath: "cleanup",
    ptt: "try",
    mic: "try",
  });
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

test("a folder import's Hebrew model, language-ID file and language are listed, and old configs compare equal", () => {
  const before = { ...cfg, hebrew: { model: "auto", dir: null }, lid: "C:\\m\\lid.bin", language: { codes: ["en"], keyboard: true } };
  const after = { ...cfg, hebrew: { model: "zipformer", dir: null }, lid: null, language: { codes: [], keyboard: false } };
  const L2 = { ...L, hebrewName: (m) => `H(${m})` };
  const ch = setupChanges(before, after, L2);
  assert.deepEqual(
    ch.map((c) => [c.key, c.from, c.to]),
    [
      ["hebrew", "H(auto)", "H(zipformer)"],
      ["lid", "lid.bin", "In the models folder"],
      ["language", "Keyboard language", "Automatic"],
    ],
  );
  assert.ok(ch.every((c) => STEP_OF[c.key] === "model"));
  assert.equal(choiceLabel("language", { ...cfg, language: { codes: ["en", "he"], keyboard: false } }, L), "en, he");
  // Settings saved before these were tracked: no change.
  assert.deepEqual(setupChanges(cfg, { ...cfg }, L), []);
  assert.deepEqual(setupChanges(after, { ...after, language: { codes: [], keyboard: false } }, L), []);
});
