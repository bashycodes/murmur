// D105: Settings → General → Send to webhook. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { looksLikeUrl, redactUrl, webhookStatus } from "../dist/lib/webhook.js";

test("off until both the URL and the shortcut are set", () => {
  assert.deepEqual(webhookStatus("", ""), { on: false, text: "Off. Set a URL and a shortcut to turn it on." });
  assert.deepEqual(webhookStatus("https://hooks.example.com/in", ""), {
    on: false,
    text: "Off until you set a shortcut.",
  });
  assert.deepEqual(webhookStatus("  ", "Ctrl+Alt+W"), { on: false, text: "Off until you set a URL." });
  const s = webhookStatus(" https://hooks.example.com/in ", "Ctrl+Alt+W");
  assert.equal(s.on, true);
  assert.match(s.text, /hold Ctrl\+Alt\+W/);
  assert.match(s.text, /https:\/\/hooks\.example\.com\/in and not pasted/);
});

test("the status never shows the query string", () => {
  const s = webhookStatus("https://hooks.example.com/in?token=s3cret", "Ctrl+Alt+W");
  assert.ok(!s.text.includes("s3cret"), s.text);
  assert.equal(redactUrl("https://h.example.com/a?k=1#f"), "https://h.example.com/a?…");
  assert.equal(redactUrl("http://10.0.0.5:5678/hook"), "http://10.0.0.5:5678/hook");
});

test("a URL without http(s):// is flagged before any save", () => {
  assert.equal(looksLikeUrl("https://hooks.example.com/in"), true);
  assert.equal(looksLikeUrl("HTTP://192.168.1.20/hook"), true);
  assert.equal(looksLikeUrl("hooks.example.com/in"), false);
  assert.equal(looksLikeUrl("ftp://example.com"), false);
  assert.equal(looksLikeUrl("https://a b"), false);
  const s = webhookStatus("hooks.example.com/in", "Ctrl+Alt+W");
  assert.equal(s.on, false);
  assert.match(s.text, /must start with https:\/\//);
});
