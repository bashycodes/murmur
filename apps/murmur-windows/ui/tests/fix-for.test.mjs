// Regression (D78): Command Mode / transform refusals name the Cloud
// provider ("… to OpenRouter", "… to LiteLLM (10.0.0.5)", "… to the cloud").
// Their Fix link must open the AI cleanup page, where the "Text around the
// cursor" switch is, for every provider. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fixFor } from "../dist/lib/fix-for.js";

const refusal = (to) =>
  `Command Mode would send the selection to ${to}: allow surrounding text there, or use a local AI model`;

test("selection refusals open the AI cleanup page for every provider", () => {
  for (const to of ["OpenRouter", "LiteLLM (10.0.0.5)", "LiteLLM (myserver.lan)", "the cloud"]) {
    assert.deepEqual(fixFor(refusal(to)), { page: "settings-cleanup", label: "Fix →" }, to);
  }
  assert.equal(
    fixFor("This transform would send the selection to LiteLLM (10.0.0.5): allow surrounding text there, or use a local AI model")?.page,
    "settings-cleanup",
  );
});

test("needs-AI text opens the AI cleanup page", () => {
  assert.deepEqual(
    fixFor("Command Mode needs AI: turn on AI cleanup (local model) or cloud text cleanup"),
    { page: "settings-cleanup", label: "Fix →" },
  );
});

test("other texts keep their pages", () => {
  assert.equal(fixFor("Cloud STT failed (timeout); used your server")?.page, "settings-openrouter");
  assert.equal(fixFor("No speech heard"), null);
});
