// node --test tools/e2e-windows/test/
import assert from "node:assert/strict";
import { test } from "node:test";
import { hebrewShare, normalize, similarity, wordDistance } from "../lib/similarity.mjs";
import { durationMs, joinWavs, readWav, writeWav } from "../lib/wav.mjs";
import { defaultHost, parseArgs, readEnvFile } from "../lib/args.mjs";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.MURMUR_E2E_HOST = "tester@laptop.example";

test("normalize drops case, punctuation, niqqud and bidi marks", () => {
  assert.equal(normalize("Hello, World!  "), "hello world");
  assert.equal(normalize("שָׁלוֹם‏ עולם."), "שלום עולם");
  assert.equal(normalize("don't"), "dont");
});

test("word distance", () => {
  assert.equal(wordDistance(["a", "b", "c"], ["a", "c"]), 1);
  assert.equal(wordDistance([], ["a"]), 1);
  assert.equal(wordDistance(["a"], ["a"]), 0);
});

test("similarity is loose about formatting, strict about words", () => {
  assert.equal(similarity("The quick brown fox.", "the quick, brown fox").score, 1);
  const s = similarity("send the weekly report today", "send the report today");
  assert.equal(s.score, 0.8);
  assert.equal(s.recall, 0.8);
  assert.equal(similarity("a b", "").score, 0);
  assert.equal(similarity("", "").score, 1);
  // Reordering costs WER but not recall.
  assert.equal(similarity("one two three", "three one two").recall, 1);
});

test("hebrewShare", () => {
  assert.equal(hebrewShare("שלום"), 1);
  assert.equal(hebrewShare("hello"), 0);
  assert.equal(hebrewShare("שלום hi"), 4 / 6);
  assert.equal(hebrewShare("123 ..."), null);
});

test("wav round trip and join", () => {
  const a = { rate: 16000, channels: 1, data: Buffer.alloc(3200, 1) }; // 100 ms
  const back = readWav(writeWav(a));
  assert.equal(back.rate, 16000);
  assert.equal(durationMs(back), 100);
  const j = joinWavs([back, back], 50);
  assert.equal(durationMs(j), 100 * 2 + 50 * 3);
});

test("args", () => {
  const a = parseArgs(["--installer", "M.exe", "--scenarios", "a,c"]);
  assert.equal(a.command, "run");
  assert.equal(a.installer, "M.exe");
  assert.deepEqual(a.scenarios, ["a", "c"]);
  assert.equal(a.restore, true);
  assert.equal(a.tries, 4);
  assert.equal(a.wait, true);
  assert.equal(parseArgs(["restore"]).command, "restore");
  const i = ["--installer", "M.exe"];
  assert.equal(parseArgs([...i, "--no-restore"]).restore, false);
  assert.equal(parseArgs([...i, "--no-wait"]).wait, false);
  assert.deepEqual(parseArgs(i).scenarios, ["a", "b", "c", "d", "e", "f"]);
  assert.deepEqual(parseArgs([...i, "--gaps", "100,250"]).gaps, [100, 250]);
  assert.deepEqual(parseArgs([...i, "--tap-ms", "90"]).taps, [90]);
  assert.deepEqual(parseArgs([...i, "--keys", "RAlt"]).keys, ["RAlt"]);
  assert.deepEqual(parseArgs([...i, "--profiles", "owner"]).profiles, ["owner"]);
  assert.equal(parseArgs([...i, "--altgr", "--layout", "he"]).layout, "he");
  assert.throws(() => parseArgs([...i, "--scenarios", "z"]), /unknown z/);
  assert.throws(() => parseArgs([...i, "--keys", "Ctrl"]), /unknown Ctrl/);
  assert.throws(() => parseArgs([...i, "--gaps", "x"]), /expected numbers/);
  assert.throws(() => parseArgs([]), /one of --installer or --installer-local/);
  assert.throws(() => parseArgs([...i, "--installer-local", "/nonexistent.exe"]), /one of/);
  assert.throws(() => parseArgs(["--installer-local", "/nonexistent.exe"]), /no such file/);
});

test("host: env, then local.env, else a clear error", () => {
  const d = mkdtempSync(join(tmpdir(), "e2e-env-"));
  const f = join(d, "local.env");
  assert.equal(defaultHost({}, f), null);
  writeFileSync(f, "# laptop\nMURMUR_E2E_HOST = 'me@box'\n");
  assert.deepEqual(readEnvFile(f), { MURMUR_E2E_HOST: "me@box" });
  assert.equal(defaultHost({}, f), "me@box");
  assert.equal(defaultHost({ MURMUR_E2E_HOST: "env@box" }, f), "env@box");
  assert.equal(parseArgs(["--installer", "M.exe"]).host, "tester@laptop.example");
  assert.equal(parseArgs(["--installer", "M.exe", "--host", "x@y"]).host, "x@y");
  assert.equal(parseArgs(["--installer", "M.exe", "--allow-downgrade"]).allowDowngrade, true);
});
