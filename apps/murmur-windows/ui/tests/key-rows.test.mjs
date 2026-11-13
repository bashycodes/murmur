// Settings → Users & keys: one date format, the active/revoked split and the
// row text. Runs on the built dist: `npm test`. Key names are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { dateTime } from "../dist/lib/format.js";
import { keyDetails, lastUsed, splitKeys } from "../dist/lib/key-rows.js";

const secs = (...a) => new Date(...a).getTime() / 1000;
const key = (over = {}) => ({
  id: "k1",
  name: "test-device-1",
  admin: false,
  created: secs(2026, 0, 5, 9, 3),
  dictations: 0,
  audio_ms: 0,
  current: false,
  ...over,
});

test("dateTime: day, month, year and time, today or not", () => {
  assert.equal(dateTime(new Date(2026, 10, 11, 22, 51).getTime()), "11 Nov 2026, 22:51");
  const today = new Date();
  today.setHours(7, 4, 0, 0);
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  assert.equal(
    dateTime(today.getTime()),
    `${today.getDate()} ${months[today.getMonth()]} ${today.getFullYear()}, 07:04`,
  );
});

test("splitKeys: active in server order, revoked latest first", () => {
  const keys = [
    key({ id: "a" }),
    key({ id: "r-old", revoked: secs(2026, 1, 1) }),
    key({ id: "b" }),
    key({ id: "r-new", revoked: secs(2026, 5, 1) }),
  ];
  const { active, revoked } = splitKeys(keys);
  assert.deepEqual(active.map((k) => k.id), ["a", "b"]);
  assert.deepEqual(revoked.map((k) => k.id), ["r-new", "r-old"]);
});

test("splitKeys: no keys", () => {
  assert.deepEqual(splitKeys([]), { active: [], revoked: [] });
});

test("lastUsed: a date, Never, or a dash for use before tracking", () => {
  assert.equal(lastUsed(key({ last_used: secs(2026, 10, 12, 12, 4) })), "12 Nov 2026, 12:04");
  assert.equal(lastUsed(key()), "Never");
  assert.equal(lastUsed(key({ dictations: 3 })), "—");
  assert.equal(lastUsed(key({ downloads: 1 })), "—");
});

test("keyDetails: build string first, then what the table leaves out", () => {
  const lines = keyDetails(
    key({
      client: "example-app 0.0.1+abcdef",
      last_seen: secs(2026, 10, 1, 8, 0),
      last_used: secs(2026, 10, 1, 7, 55),
      audio_ms: 90_000,
      downloads: 2,
      download_bytes: 3_500_000,
      revoked: secs(2026, 10, 2, 10, 30),
    }),
  ).split("\n");
  assert.deepEqual(lines, [
    "example-app 0.0.1+abcdef",
    "Created 5 Jan 2026, 09:03",
    "Last connected 1 Nov 2026, 08:00",
    "Audio 1.5 min",
    "Model downloads 2 (3.5 MB)",
    "Revoked 2 Nov 2026, 10:30",
  ]);
});

test("keyDetails: an unused key", () => {
  assert.equal(keyDetails(key()).split("\n")[0], "Not used yet");
  assert.match(keyDetails(key({ dictations: 1 })), /before the server tracked it/);
});
