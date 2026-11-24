// D34: Settings → General → Languages, auto-detect without the language-ID model. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { autoDetects, forcedLanguage, lidWarning } from "../dist/lib/lid-warning.js";

const base = {
  keyboard: false,
  languages: [],
  hebrewModel: true,
  lidPresent: false,
  serverHebrew: false,
  target: { canStart: true, busy: false, missing: 31_000_000 },
};

test("auto-detect with a Hebrew model and no language-ID model warns, with a one-click download", () => {
  const w = lidWarning(base);
  assert.equal(w.show, true);
  assert.match(w.text, /treated as English/);
  assert.match(w.text, /\(31 MB\)/);
  assert.equal(w.download, "Download 31 MB…");
  assert.equal(w.go, undefined);
});

test("no warning once the model is there, auto-detect is off or there is no Hebrew model", () => {
  assert.deepEqual(lidWarning({ ...base, lidPresent: true }), { show: false });
  assert.deepEqual(lidWarning({ ...base, lidPresent: undefined }), { show: false });
  assert.deepEqual(lidWarning({ ...base, languages: ["en"] }), { show: false });
  assert.deepEqual(lidWarning({ ...base, languages: ["he"] }), { show: false });
  assert.deepEqual(lidWarning({ ...base, keyboard: true }), { show: false });
  assert.deepEqual(lidWarning({ ...base, hebrewModel: false }), { show: false });
});

test("a build without the downloader warns without the button and points to Speech to text", () => {
  const w = lidWarning({ ...base, target: { ...base.target, canStart: false } });
  assert.equal(w.show, true);
  assert.equal(w.download, undefined);
  assert.deepEqual(w.go, { page: "settings-speech", label: "Get it under Speech to text →" });
});

test("a Hebrew model from the server points to the server; nothing to fetch points to Speech to text", () => {
  const s = lidWarning({ ...base, serverHebrew: true, target: undefined });
  assert.equal(s.download, undefined);
  assert.equal(s.go.page, "settings-server");
  assert.match(s.text, /\(32 MB\)/);
  // The catalogue says complete, yet the resolved file is missing (a server path that vanished).
  const c = lidWarning({ ...base, target: { ...base.target, missing: 0 } });
  assert.equal(c.download, undefined);
  assert.equal(c.go.page, "settings-speech");
});

test("while it downloads, the warning says so and offers no second button", () => {
  const w = lidWarning({ ...base, target: { ...base.target, busy: true } });
  assert.equal(w.show, true);
  assert.equal(w.download, undefined);
  assert.equal(w.go, undefined);
  assert.match(w.text, /Downloading/);
});

test("auto-detect follows murmur-core's forced-language rule and LID candidates", () => {
  // Pinned against SessionConfig::forced_language / Routes::lid_candidates.
  assert.equal(forcedLanguage([]), undefined);
  assert.equal(forcedLanguage(["auto"]), undefined);
  assert.equal(forcedLanguage(["en"]), "en");
  assert.equal(forcedLanguage(["en", "he"]), undefined);
  assert.equal(autoDetects(false, []), true);
  assert.equal(autoDetects(false, ["auto"]), true);
  assert.equal(autoDetects(false, ["auto", "he"]), true);
  assert.equal(autoDetects(false, ["en", "he-IL"]), true);
  assert.equal(autoDetects(false, ["en", "iw"]), true);
  assert.equal(autoDetects(false, ["en", "de"]), false);
  assert.equal(autoDetects(false, ["iw"]), false);
  assert.equal(autoDetects(false, ["en"]), false);
  assert.equal(autoDetects(true, []), false);
});

test("languages = [\"auto\"] warns like an empty list", () => {
  assert.equal(lidWarning({ ...base, languages: ["auto"] }).show, true);
});
