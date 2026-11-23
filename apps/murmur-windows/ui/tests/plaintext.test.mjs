// D100: the per-server plaintext opt-in. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { isPlaintextRefusal, optInFor, sameHost } from "../dist/lib/plaintext.js";

test("the opt-in stays only with the same server address", () => {
  assert.equal(optInFor("30.1.2.3", true, "30.1.2.3"), true);
  assert.equal(optInFor("asr.corp.example.com", true, " ASR.corp.example.com "), true);
  assert.equal(optInFor("fd00::1", true, "[fd00::1]"), true);
  assert.equal(optInFor("30.1.2.3", true, "30.1.2.4"), false, "another server starts without it");
  assert.equal(optInFor("30.1.2.3", false, "30.1.2.3"), false);
  assert.equal(optInFor("", true, "30.1.2.3"), false);
  assert.equal(sameHost("a", "b"), false);
});

test("murmur-remote's plaintext refusals are recognised", () => {
  assert.ok(isPlaintextRefusal("box.example:7313: plaintext refused: 203.0.113.5 is not a private address (enable TLS)"));
  assert.ok(isPlaintextRefusal("connect_failed: plaintext to public address (enable TLS)"));
  assert.ok(!isPlaintextRefusal("connect_failed: refused"));
  assert.ok(!isPlaintextRefusal(undefined));
});
