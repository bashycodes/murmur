// Fewer steps to an AI provider: cloud cleanup needs no cursor context, so
// the provider card asks about it inline, only while cloud cleanup is on
// and the "Text around the cursor" switch is off. Runs on the built dist:
// `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { selectionAsk } from "../dist/lib/selection-ask.js";

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
