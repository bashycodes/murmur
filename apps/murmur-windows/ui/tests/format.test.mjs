// D112: a server lockout ("throttled") reads as its own sentence with the
// wait, never as "can't reach the server". Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { serverBadge, serverFailText, throttledLeft, throttledText } from "../dist/lib/format.js";

const hint = (code) => (code === "connect_failed" ? "is murmur-server running?" : "");
const later = "Too many wrong keys from this address. Try again later. Your key isn't checked until then.";

test("the lockout sentence names the wait when the server sent one", () => {
  assert.equal(
    throttledText(42),
    "Too many wrong keys from this address. Try again in 42 s. Your key isn't checked until then.",
  );
  assert.equal(throttledText(null), later);
  assert.equal(throttledText(undefined), later);
  assert.equal(throttledText(0), "Too many wrong keys from this address. The wait is over: try again.");
});

test("the wait counts down from the probe time", () => {
  const h = { retry_after_s: 60, checked_at_ms: 1_000_000 };
  assert.equal(throttledLeft(h, 1_000_000), 60);
  assert.equal(throttledLeft(h, 1_045_500), 15);
  assert.equal(throttledLeft(h, 1_090_000), 0);
  assert.equal(throttledLeft({ retry_after_s: 9 }, 5), 9);
  assert.equal(throttledLeft({ retry_after_s: null, checked_at_ms: 1 }, 5), null);
});

test("a throttled Test result is the sentence with the wait left, not the can't-reach hint", () => {
  const h = { error: "throttled", message: "x", retry_after_s: 30, checked_at_ms: 10_000 };
  assert.equal(serverFailText(h, hint, 20_000), throttledText(20));
  assert.equal(serverFailText({ error: "throttled", message: null }, hint), later);
});

test("other failures keep code, message and hint", () => {
  assert.equal(
    serverFailText({ error: "connect_failed", message: "refused" }, hint),
    "connect_failed: refused — is murmur-server running?",
  );
  assert.equal(serverFailText({ error: null, message: null }, hint), "error");
});

test("the server badge says Locked out for a lockout", () => {
  assert.deepEqual(serverBadge({ ok: false, error: "throttled" }), ["Locked out", "warning"]);
  assert.deepEqual(serverBadge({ ok: false, error: "connect_failed" }), ["Unreachable", "destructive"]);
  assert.deepEqual(serverBadge({ ok: true, error: null }), ["Connected", "success"]);
});
