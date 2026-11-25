// D112: a server lockout ("throttled") reads as its own sentence with the
// wait, never as "can't reach the server". Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { serverFailText, throttledText } from "../dist/lib/format.js";

const hint = (code) => (code === "connect_failed" ? "is murmur-server running?" : "");

test("the lockout sentence names the wait when the server sent one", () => {
  assert.equal(throttledText(42), "Too many wrong keys from this address. Try again in 42 s.");
  assert.equal(throttledText(null), "Too many wrong keys from this address. Try again later.");
  assert.equal(throttledText(undefined), "Too many wrong keys from this address. Try again later.");
});

test("a throttled Test result is the sentence, not the can't-reach hint", () => {
  const h = { error: "throttled", message: "Too many wrong keys from this address. Try again in 9 s.", retry_after_s: 9 };
  assert.equal(serverFailText(h, hint), "Too many wrong keys from this address. Try again in 9 s.");
  assert.equal(serverFailText({ error: "throttled", message: null }, hint), throttledText());
});

test("other failures keep code, message and hint", () => {
  assert.equal(
    serverFailText({ error: "connect_failed", message: "refused" }, hint),
    "connect_failed: refused — is murmur-server running?",
  );
  assert.equal(serverFailText({ error: null, message: null }, hint), "error");
});
