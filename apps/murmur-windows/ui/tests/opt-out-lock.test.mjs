// An opted-out feature's page is locked (Transforms first). Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { applyOptOut, optOutMarkup, wireOptOut } from "../dist/lib/opt-out-lock.js";

/** The three pieces optOutMarkup("tf", …) names, as plain objects. */
function fakePage() {
  const clicks = [];
  const els = {
    tf_off: { hidden: true },
    tf_region: { disabled: false },
    tf_turn_on: { addEventListener: (type, fn) => clicks.push([type, fn]) },
  };
  return { els, find: (id) => els[id], click: () => clicks.filter(([t]) => t === "click").forEach(([, fn]) => fn()) };
}

test("the markup: a banner hidden until locked, with a turn-on button, and a fieldset around the controls", () => {
  const m = optOutMarkup("tf", "Transforms", "Your transforms are kept.");
  assert.match(m.banner, /id="tf_off"[^>]*hidden/);
  assert.match(m.banner, /Transforms is off\./);
  assert.match(m.banner, /<button type="button"[^>]*id="tf_turn_on">Turn on Transforms<\/button>/);
  assert.match(m.open, /^<fieldset [^>]*id="tf_region">$/);
  assert.equal(m.close, "</fieldset>");
});

test("off: the controls are disabled and the banner shows; on: unlocked, banner gone", () => {
  const view = { banner: { hidden: true }, region: { disabled: false } };
  applyOptOut(view, false);
  assert.deepEqual(view, { banner: { hidden: false }, region: { disabled: true } });
  applyOptOut(view, true);
  assert.deepEqual(view, { banner: { hidden: true }, region: { disabled: false } });
});

test("the banner's button turns the feature on, and the page unlocks at once", () => {
  const page = fakePage();
  let on = false;
  const apply = wireOptOut("tf", page.find, () => {
    on = true;
    apply(on); // the page re-renders from the saved state
  });
  apply(on);
  assert.equal(page.els.tf_region.disabled, true, "locked while opted out");
  assert.equal(page.els.tf_off.hidden, false);
  page.click();
  assert.equal(on, true, "turned on through the feature's own path");
  assert.equal(page.els.tf_region.disabled, false, "unlocked without a reload");
  assert.equal(page.els.tf_off.hidden, true);
  apply(false);
  assert.equal(page.els.tf_region.disabled, true, "opting out again locks it again");
});
