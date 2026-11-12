// Command Mode's answer to a question shows as a card in the Murmur bar's
// bubble, never with a "Fix →" link (its text is the AI's). Runs on the
// built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { bubbleFor } from "../dist/lib/overlay-bubble.js";

test("an answer is a card without a fix link", () => {
  const text = "It asks the server team to move the meeting.\nNothing else.";
  assert.deepEqual(bubbleFor({ state: "answer", message: text }), { text, kind: "answer", fix: null });
});

test("errors and notices keep their dot and fix link", () => {
  assert.deepEqual(bubbleFor({ state: "error", message: "Couldn't reach your server" }), {
    text: "Couldn't reach your server",
    kind: "error",
    fix: { page: "settings-server", label: "Fix →" },
  });
  assert.equal(bubbleFor({ state: "notice", message: "No speech heard" })?.kind, "notice");
  assert.equal(bubbleFor({ state: "notice", message: "No speech heard" })?.fix, null);
});

test("states without a message have no bubble of their own", () => {
  for (const state of ["resting", "active_ptt", "processing", "command_processing"]) {
    assert.equal(bubbleFor({ state }), null, state);
  }
  assert.equal(bubbleFor({ state: "answer", message: "" }), null);
});
