// Settings → Developer's "Last dictation" (UX review on 1.3.0): before this
// run has dictated it shows history's newest plain dictation, in the same
// engine words as the live line. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  engineWord,
  historyLastText,
  latestOnly,
  NO_DICTATION,
  pickDictation,
} from "../dist/lib/last-dictation.js";

const TS = new Date(2026, 10, 3, 14, 5).getTime();
const row = (over = {}) => ({ ts: TS, app: "notepad.exe", words: 12, route: "server", ...over });

test("live engine words", () => {
  assert.equal(engineWord("remote"), "server");
  assert.equal(engineWord("local"), "client");
  assert.equal(engineWord("cloud"), "cloud");
  assert.equal(engineWord(null), "none");
});

test("history routes use the live engine words", () => {
  const text = (route) => historyLastText(row({ route }), "en-US").split(" · ")[3];
  assert.equal(text("pc"), "client");
  assert.equal(text("server"), "server");
  assert.equal(text("cloud"), "cloud");
});

test("the history line: when, app without .exe, words, engine, note", () => {
  const when = new Date(TS).toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
  assert.equal(
    historyLastText(row(), "en-US"),
    `${when} · notepad · 12 words · server · timings show after the next dictation`,
  );
  assert.equal(
    historyLastText(row({ app: "", words: 1, route: null }), "en-US"),
    `${when} · unknown app · 1 word · timings show after the next dictation`,
  );
  assert.equal(historyLastText(null), NO_DICTATION);
});

test("Command Mode and transform rows are skipped", () => {
  const rows = [row({ command: true, words: 1 }), row({ transform: "Email", words: 2 }), row({ words: 3 }), row({ words: 4 })];
  assert.equal(pickDictation(rows)?.words, 3);
  assert.equal(pickDictation(rows.slice(0, 2)), null);
  assert.equal(pickDictation([]), null);
  assert.equal(pickDictation([row({ transform: null, command: false, words: 5 })])?.words, 5);
});

test("only the newest update may write", () => {
  const g = latestOnly();
  const historyRead = g.next(); // no live dictation yet: history read starts
  const liveDictation = g.next(); // a dictation arrives before history answers
  assert.equal(g.isLatest(historyRead), false);
  assert.equal(g.isLatest(liveDictation), true);
});
