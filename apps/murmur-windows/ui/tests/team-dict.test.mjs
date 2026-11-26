// The team dictionary as the Dictionary page and Users & keys show it.
// Runs on the built dist: `npm test`. Words are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { teamEntryFrom, teamNote, teamRows, teamSummary, wordKey } from "../dist/lib/team-dict.js";

const team = [
  { word: "zephyr", replacement: "ZephyrCo" },
  { word: "Quillon", replacement: null },
  { word: "acme  corp", replacement: "AcmeCorp" },
];

test("wordKey ignores case and spacing", () => {
  assert.equal(wordKey("  Acme   Corp "), "acme corp");
});

test("teamRows: server order, heard → word, personal words win", () => {
  const personal = [
    { word: "ZEPHYR", replacement: "Zephyr" },
    { word: "quillon", replacement: "quillon text", isSnippet: true },
  ];
  assert.deepEqual(teamRows(team, personal), [
    { heard: "zephyr", word: "ZephyrCo", key: "zephyr", overridden: true },
    // A snippet is not a dictionary word: the team word is used.
    { heard: "", word: "Quillon", key: "Quillon", overridden: false },
    { heard: "acme  corp", word: "AcmeCorp", key: "acme  corp", overridden: false },
  ]);
});

test("teamSummary counts the words and the ones yours replace", () => {
  assert.equal(teamSummary([], []), "Your server's team dictionary is empty.");
  assert.equal(teamSummary(team.slice(1, 2), []), "1 word from your server.");
  assert.equal(
    teamSummary(team, [{ word: "Zephyr" }]),
    "3 words from your server. 1 also in your dictionary: yours is used.",
  );
});

test("teamEntryFrom: heard as becomes the word, the word its replacement", () => {
  assert.deepEqual(teamEntryFrom(" Quillon ", ""), { word: "Quillon", replacement: "" });
  assert.deepEqual(teamEntryFrom("AcmeCorp", " acme corp "), { word: "acme corp", replacement: "AcmeCorp" });
  assert.equal(teamEntryFrom("  ", "heard"), "Fill in the word.");
});

test("teamNote: the backend's reason wins, so a refused key never reads as a saved copy", () => {
  // src/team_dictionary.rs KEY_REFUSED: the 401 cleared the cached list.
  const refused = "The server refused this app's key, so no team list is in use.";
  assert.deepEqual(teamNote({ configured: true, entries: [], error: refused }, []), { text: refused, err: true });
  assert.deepEqual(teamNote({ configured: true, entries: team.slice(1, 2) }, []), {
    text: "1 word from your server.",
    err: false,
  });
});
