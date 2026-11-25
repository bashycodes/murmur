// Fewer steps to an AI provider: cloud cleanup needs no cursor context, so
// the provider card asks about it inline, only while cloud cleanup is on
// and the "Text around the cursor" switch is off. Runs on the built dist:
// `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { contextSwitches, selectionAsk, wireSelectionAsk } from "../dist/lib/selection-ask.js";

/** The ask's row and the cleanup form's switches, as plain objects. */
function fakeCard() {
  let click = () => {};
  const els = {
    or_cleanup_sel_on: { addEventListener: (_t, fn) => (click = fn) },
    or_cleanup_sel_row: { hidden: true },
    or_cleanup_sel_ask: { textContent: "" },
    or_cleanup_send_surrounding_text: { checked: false },
    or_cleanup_send_screen_words: { checked: false },
    or_cleanup_send_app_name: { checked: true },
  };
  return { els, find: (id) => els[id], click: () => click() };
}

test("Also send it saves send_surrounding_text=true and nothing else changes", () => {
  const card = fakeCard();
  const saved = [];
  const show = wireSelectionAsk(card.find, () => saved.push(contextSwitches(card.find)));
  show({ cloudOn: true, sendSurrounding: false, provider: "openrouter" });
  assert.equal(card.els.or_cleanup_sel_row.hidden, false);
  assert.match(card.els.or_cleanup_sel_ask.textContent, /text around your cursor/);
  card.click();
  assert.deepEqual(saved, [{ send_surrounding_text: true, send_screen_words: false, send_app_name: true }]);
});

test("the ask hides once the saved settings send the text around the cursor", () => {
  const card = fakeCard();
  const show = wireSelectionAsk(card.find, () => {});
  show({ cloudOn: true, sendSurrounding: false, provider: "custom" });
  assert.equal(card.els.or_cleanup_sel_row.hidden, false);
  show({ cloudOn: true, sendSurrounding: true, provider: "custom" });
  assert.equal(card.els.or_cleanup_sel_row.hidden, true);
  assert.equal(card.els.or_cleanup_sel_ask.textContent, "");
  show({ cloudOn: false, sendSurrounding: false, provider: "custom" });
  assert.equal(card.els.or_cleanup_sel_row.hidden, true, "cloud off: nothing to ask");
});

test("no ask while cloud cleanup is off", () => {
  assert.equal(selectionAsk({ cloudOn: false, sendSurrounding: false, provider: "openrouter" }), null);
  assert.equal(selectionAsk({ cloudOn: false, sendSurrounding: true, provider: "custom" }), null);
});

test("no ask once the text around the cursor may go", () => {
  assert.equal(selectionAsk({ cloudOn: true, sendSurrounding: true, provider: "openrouter" }), null);
});

test("cloud on, context off: dictation works, the ask names the destination", () => {
  const or = selectionAsk({ cloudOn: true, sendSurrounding: false, provider: "openrouter" });
  assert.match(or, /Dictation cleanup doesn't need it\.$/);
  assert.match(or, /selected text in the cloud \(Command Mode, transforms\)/);
  assert.match(or, /text around your cursor/);
  const own = selectionAsk({ cloudOn: true, sendSurrounding: false, provider: "custom" });
  assert.match(own, /selected text on your own AI server/);
  assert.match(selectionAsk({ cloudOn: true, sendSurrounding: false, provider: undefined }), /in the cloud/);
});
