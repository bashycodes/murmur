// Regression (Windows 1.1.2): the custom server key's Save reloaded the
// settings while the address field's save (fired by the same click) was
// still running, and the reload showed the old provider. The cleanup
// card's writes now share one queue. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Serial } from "../dist/lib/serial.js";

test("a reload queued after a save sees the saved value", async () => {
  const q = new Serial();
  let stored = "openrouter";
  let release;
  const gate = new Promise((r) => (release = r));
  // The address field's change (fired by the click on Save) saves Custom...
  const save = q.run(async () => {
    await gate;
    stored = "custom";
  });
  // ...and the key's Save reloads after it, even though it was started now.
  const shown = q.run(async () => stored);
  release();
  await save;
  assert.equal(await shown, "custom");
});

test("jobs run one at a time, in order", async () => {
  const q = new Serial();
  const log = [];
  let running = 0;
  const job = (n) => async () => {
    running++;
    assert.equal(running, 1);
    await new Promise((r) => setTimeout(r, 5 - n));
    log.push(n);
    running--;
  };
  await Promise.all([q.run(job(1)), q.run(job(2)), q.run(job(3))]);
  assert.deepEqual(log, [1, 2, 3]);
});

test("a failed job does not block later ones", async () => {
  const q = new Serial();
  await assert.rejects(q.run(async () => { throw new Error("bad url"); }));
  assert.equal(await q.run(async () => 1), 1);
});
