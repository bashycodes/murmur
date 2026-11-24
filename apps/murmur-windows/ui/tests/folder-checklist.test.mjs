// The "Use model files on this PC" / "Import from folder…" checklist rows.
import { test } from "node:test";
import assert from "node:assert/strict";
import { canUse, downloadTarget, importLine, notes, pickOf, rowView, scanLine } from "../dist/lib/folder-checklist.js";

const found = (title, installed = false) => ({ state: "found", id: "x", title, size: 600e6, folder: "library/x", installed });
const missing = { state: "missing" };
const base = { use: true, skipped: false, have: false, canStart: true, pct: null };
const dl = { id: "en", size: 640e6, label: "Download 640 MB" };

const CATALOG = [
  { id: "en", title: "Parakeet", size: 640e6, engine: "parakeet", recommended: true, installed: false },
  { id: "wb", title: "Whisper base", size: 150e6, engine: "whisper", recommended: false, installed: false },
  { id: "he", title: "Hebrew fast", size: 120e6, language: "he", engine: "sherpa_online", recommended: true, installed: false },
  { id: "llm", title: "Cleanup", size: 900e6, engine: "llm", recommended: true, installed: false },
];

const scan = (e, h, l) => ({ folder: "D:\\models", english: e, hebrew: h, lid: l, others: [], problems: [], unrecognised: [] });

test("a found row shows name, size, folder and a tick", () => {
  const v = rowView({ ...base, slot: found("Parakeet TDT") });
  assert.equal(v.name, "Parakeet TDT");
  assert.match(v.detail, /600 MB · in library\/x/);
  assert.equal(v.badge, "Found");
  assert.equal(v.tone, "ok");
  assert.ok(v.tick);
  assert.equal(v.download, null);
  assert.ok(!v.skip);
});

test("an unticked found row says it won't be used", () => {
  const v = rowView({ ...base, slot: found("Parakeet"), use: false });
  assert.match(v.detail, /won't be used/);
  assert.ok(v.tick);
  assert.equal(v.tone, "muted");
});

test("a found row already installed says so", () => {
  assert.match(rowView({ ...base, slot: found("P", true) }).detail, /already on this PC/);
});

test("a missing row offers Download and Skip", () => {
  const v = rowView({ ...base, slot: missing, download: dl });
  assert.equal(v.badge, "Not found");
  assert.equal(v.download, "Download 640 MB");
  assert.ok(v.skip);
  assert.match(v.detail, /Download it, or skip it/);
});

test("a missing row without a downloader offers only Skip", () => {
  const v = rowView({ ...base, slot: missing, download: undefined, canStart: false });
  assert.equal(v.download, null);
  assert.ok(v.skip);
  assert.match(v.detail, /Settings → Speech to text/);
});

test("Skip, a running download and a model already on this PC", () => {
  assert.equal(rowView({ ...base, slot: missing, download: dl, skipped: true }).badge, "Skipped");
  const d = rowView({ ...base, slot: missing, download: dl, pct: 42 });
  assert.equal(d.badge, "Downloading 42%");
  assert.equal(d.download, null);
  const h = rowView({ ...base, slot: missing, download: dl, have: true });
  assert.equal(h.badge, "Ready");
  assert.ok(!h.skip);
});

test("a problem row is a warning with its message, and can be downloaded instead", () => {
  const v = rowView({ ...base, slot: { state: "problem", title: "Hebrew fast", message: "encoder.onnx is 3 bytes, needs 9" }, download: dl });
  assert.equal(v.tone, "warn");
  assert.equal(v.badge, "Can't use");
  assert.match(v.detail, /3 bytes/);
  assert.ok(v.download);
});

test("after Use these: imported, failed, and not used", () => {
  const ok = rowView({ ...base, slot: found("P"), result: { state: "imported", id: "x", title: "Parakeet" } });
  assert.equal(ok.badge, "Ready");
  assert.ok(!ok.tick);
  const bad = rowView({ ...base, slot: found("P"), result: { state: "failed", message: "fingerprint differs" }, download: dl });
  assert.equal(bad.tone, "err");
  assert.match(bad.detail, /fingerprint/);
  assert.ok(bad.download && bad.skip);
  const off = rowView({ ...base, slot: found("P"), use: false, result: { state: "skipped" } });
  assert.equal(off.badge, "Not used");
  assert.ok(!off.tick);
});

test("Use these needs a ticked found row", () => {
  const all = { english: true, hebrew: true, lid: true };
  assert.ok(!canUse(null, all));
  assert.ok(!canUse(scan(missing, missing, missing), all));
  const s = scan(found("E"), missing, found("L"));
  assert.ok(canUse(s, all));
  assert.ok(!canUse(s, { english: false, hebrew: true, lid: false }));
  assert.deepEqual(pickOf(s, all), { english: true, hebrew: false, lid: true });
});

test("the scan line counts what was found", () => {
  assert.match(scanLine(scan(found("E"), found("H"), found("L"))), /All three/);
  assert.match(scanLine(scan(missing, missing, missing)), /None of the three/);
  assert.match(scanLine(scan(found("E"), missing, missing)), /Found 1 of 3/);
});

test("other models, problems and unknown files are all listed", () => {
  const s = {
    ...scan(missing, missing, missing),
    others: [{ id: "llm", title: "Cleanup", size: 900e6, kind: "cleanup" }],
    problems: [{ title: "Whisper base", message: "a file is missing" }],
    unrecognised: [{ path: "stuff.zip", size: 5e6, why: "a .zip: unzip it into this folder first" }],
  };
  const n = notes(s);
  assert.equal(n.length, 3);
  assert.match(n[0], /Cleanup.*AI cleanup/);
  assert.match(n[1], /Whisper base: a file is missing/);
  assert.match(n[2], /stuff\.zip.*unzip/);
});

test("download targets: recommended English, recommended Hebrew, language ID with it", () => {
  assert.equal(downloadTarget("english", CATALOG).id, "en");
  assert.equal(downloadTarget("hebrew", CATALOG).id, "he");
  const lid = downloadTarget("lid", CATALOG);
  assert.equal(lid.id, "he");
  assert.match(lid.label, /Download with Hebrew fast/);
  const own = CATALOG.map((c) => (c.id === "he" ? { ...c, installed: true, complete: false, missing: 32e6 } : c));
  const l2 = downloadTarget("lid", own);
  assert.equal(l2.size, 32e6);
  assert.equal(l2.label, "Download 32 MB");
  assert.equal(downloadTarget("english", []), undefined);
});

test("the line after Use these", () => {
  const imp = { state: "imported", id: "x", title: "X" };
  assert.match(importLine({ english: imp, hebrew: imp, lid: imp, auto_detect: true, cancelled: false }).text, /Detect automatically/);
  const partial = importLine({ english: imp, hebrew: { state: "failed", message: "m" }, lid: { state: "skipped" }, auto_detect: false, cancelled: false });
  assert.match(partial.text, /Imported 1 of 2/);
  assert.ok(!partial.ok);
  assert.match(importLine({ english: { state: "failed", message: "m" }, hebrew: { state: "skipped" }, lid: { state: "skipped" }, auto_detect: false, cancelled: false }).text, /Nothing was imported/);
  assert.match(importLine({ english: imp, hebrew: imp, lid: imp, auto_detect: false, cancelled: true }).text, /cancelled/);
});
