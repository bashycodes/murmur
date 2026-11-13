// node --test tools/e2e-windows/test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { RECOMMENDED_ID, filesArg, gradePractice, recommendedEntry, wentActive } from "../lib/onboarding.mjs";
import { encodeParams, isStateChanging } from "../lib/stepmark.mjs";

const entry = {
  id: RECOMMENDED_ID,
  dest: "X:\\profile\\AppData\\Roaming\\Murmur\\models\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
  installed: false,
  files: [
    { name: "encoder.int8.onnx", size: 652184296, extra: false },
    { name: "tokens.txt", size: 9384, extra: false },
    { name: "ggml-tiny-q5_1.bin", size: 32152673, extra: true },
  ],
};

test("the recommended id is the one the onboarding page offers", () => {
  const page = readFileSync(new URL("../../../apps/murmur-windows/ui/src/onboarding/index.ts", import.meta.url), "utf8");
  assert.match(page, new RegExp(`const PARAKEET_ID = "${RECOMMENDED_ID}"`));
});

test("recommendedEntry picks the row's catalogue entry", () => {
  assert.equal(recommendedEntry([{ id: "whisper-base" }, entry]), entry);
  assert.equal(recommendedEntry([{ id: "whisper-base" }]), null);
  assert.equal(recommendedEntry(undefined), null);
});

test("filesArg lists the model's own files with their sizes, not the extras", () => {
  assert.equal(filesArg(entry), "encoder.int8.onnx:652184296,tokens.txt:9384");
  assert.throws(() => filesArg({ id: "x", files: [] }), /no files/);
  assert.throws(() => filesArg({ id: "x", files: [{ name: "a b,c", size: 1 }] }), /unexpected/);
});

test("models.ps1 runs as a step and its arguments survive the encoding", () => {
  assert.equal(isStateChanging("models.ps1", ["-Action", "seed"]), true);
  assert.equal(isStateChanging("models.ps1", ["-Action", "stash"]), true);
  const p = JSON.parse(Buffer.from(encodeParams(["-Action", "seed", "-Dest", entry.dest, "-Files", filesArg(entry)]), "base64").toString("utf8"));
  assert.deepEqual(p, { Action: "seed", Dest: entry.dest, Files: "encoder.int8.onnx:652184296,tokens.txt:9384" });
});

test("the pill went active on push-to-talk or a lock, not on idle or processing", () => {
  assert.equal(wentActive(["idle", "active_ptt", "processing", "idle"]), true);
  assert.equal(wentActive(["active_locked"]), true);
  assert.equal(wentActive(["idle", "processing"]), false);
  assert.equal(wentActive([]), false);
  assert.equal(wentActive(undefined), false);
});

test("practice passes on text plus an active pill, whatever the words", () => {
  // A mishearing: the key worked, the words are off. Info only.
  const g = gradePractice({ text: "Send the weakly report to day.", states: ["idle", "active_ptt", "processing", "idle"], expect: "Send the weekly report today." });
  assert.equal(g.ok, true);
  assert.ok(g.word_score < 1);
  assert.equal(gradePractice({ text: "send the weekly report today", states: ["active_ptt"], expect: "Send the weekly report today." }).word_score, 1);
});

test("practice fails on an empty box or a pill that never went active", () => {
  const empty = gradePractice({ text: "  ", states: ["idle", "active_ptt", "idle"], expect: "hello there" });
  assert.equal(empty.ok, false);
  assert.equal(empty.text_ok, false);
  assert.equal(empty.overlay_active, true);
  const still = gradePractice({ text: "hello there", states: ["idle"], expect: "hello there" });
  assert.equal(still.ok, false);
  assert.equal(still.overlay_active, false);
  assert.equal(gradePractice({ text: undefined, states: undefined, expect: "x" }).ok, false);
});
