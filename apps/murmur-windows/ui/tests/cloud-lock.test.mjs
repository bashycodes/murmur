// The provider cards are locked, not just dimmed, while "Use the cloud" is
// off (Speech → Cloud, AI cleanup → Cloud): every card control sits in the
// disabled fieldset, the switch stays outside it, and the banner's button
// runs the switch's flow. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { CLEANUP_SWITCH, STT_SWITCH, wireCloudLocks } from "../dist/lib/cloud-lock.js";
import { settingsSpeechPage } from "../dist/pages/settings/speech.js";
import { settingsCleanupPage } from "../dist/pages/settings/cleanup.js";

/** The parts of `html` before and inside the fieldset `id`. */
function region(html, id) {
  const open = html.indexOf(`id="${id}"`);
  assert.ok(open > 0, `${id} missing`);
  const close = html.indexOf("</fieldset>", open);
  assert.ok(close > open, `${id} not closed`);
  return { inside: html.slice(open, close), before: html.slice(0, open) };
}

function locksControls(html, lock, ids, sw) {
  assert.match(html, new RegExp(`<div class="lw-optout" id="${lock}_off" hidden>`));
  const r = region(html, `${lock}_region`);
  for (const id of ids) assert.match(r.inside, new RegExp(`id="${id}"`), id);
  assert.doesNotMatch(r.inside, new RegExp(`id="${sw}"`), "the switch stays usable");
  assert.match(r.before, new RegExp(`id="${sw}"`));
}

test("Speech: the cloud card's controls are in the lock, the switch is not", () => {
  locksControls(
    settingsSpeechPage.template,
    "or_stt_lock",
    ["or_stt_send_dictionary", "or_stt_language", "or_stt_path", "or_stt_change", "or_stt_test"],
    STT_SWITCH,
  );
});

test("AI cleanup: the cloud card's controls are in the lock, the switch is not", () => {
  locksControls(
    settingsCleanupPage.template,
    "or_cleanup_lock",
    ["cu_model", "or_cleanup_change", "or_cleanup_sort", "or_cleanup_test", "or_cleanup_sel_on"],
    CLEANUP_SWITCH,
  );
});

function fakePage() {
  const els = {};
  const clicks = {};
  const focused = [];
  for (const p of ["or_stt_lock", "or_cleanup_lock"]) {
    els[`${p}_off`] = { hidden: true };
    els[`${p}_region`] = { disabled: false };
    els[`${p}_turn_on`] = { addEventListener: (_t, fn) => (clicks[p] = fn) };
  }
  for (const s of [STT_SWITCH, CLEANUP_SWITCH]) els[s] = { focus: () => focused.push(s) };
  return { els, clicks, focused, find: (id) => els[id] };
}

test("off: locked with the banner; on: unlocked, banner gone", () => {
  const p = fakePage();
  const locks = wireCloudLocks(p.find, { stt: () => {}, cleanup: () => {} });
  locks.stt(false);
  locks.cleanup(false);
  for (const id of ["or_stt_lock", "or_cleanup_lock"]) {
    assert.equal(p.els[`${id}_off`].hidden, false, id);
    assert.equal(p.els[`${id}_region`].disabled, true, id);
  }
  locks.stt(true);
  assert.equal(p.els.or_stt_lock_off.hidden, true);
  assert.equal(p.els.or_stt_lock_region.disabled, false);
  assert.equal(p.els.or_cleanup_lock_region.disabled, true, "each card has its own lock");
  locks.cleanup(true);
  assert.equal(p.els.or_cleanup_lock_off.hidden, true);
  assert.equal(p.els.or_cleanup_lock_region.disabled, false);
});

/** A consent flow the test answers: `answer(true)` = agreed and saved. */
function pending() {
  let answer;
  const promise = new Promise((r) => (answer = r));
  return { promise, answer };
}

test("Turn on focuses the switch only once the consent flow ends with it on", async () => {
  const p = fakePage();
  for (const s of ["or_stt_lock", "or_cleanup_lock"]) p.els[`${s}_turn_on`].focus = () => p.focused.push(`${s}_turn_on`);
  const flows = { stt: [], cleanup: [] };
  wireCloudLocks(p.find, {
    stt: () => {
      const f = pending();
      flows.stt.push(f);
      return f.promise;
    },
    cleanup: () => {
      const f = pending();
      flows.cleanup.push(f);
      return f.promise;
    },
  });
  p.clicks.or_cleanup_lock();
  assert.equal(flows.cleanup.length, 1, "that card's flow runs");
  assert.equal(flows.stt.length, 0);
  assert.deepEqual(p.focused, [], "nothing moves while the dialog is open");
  flows.cleanup[0].answer(true);
  await flows.cleanup[0].promise;
  await Promise.resolve();
  assert.deepEqual(p.focused, [CLEANUP_SWITCH]);

  p.clicks.or_stt_lock();
  flows.stt[0].answer(false);
  await flows.stt[0].promise;
  await Promise.resolve();
  assert.deepEqual(p.focused, [CLEANUP_SWITCH, "or_stt_lock_turn_on"], "cancelled: back to the banner's button");
  assert.notEqual(p.focused.at(-1), STT_SWITCH);
});
