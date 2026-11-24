// D109: the shortcut a new transform is offered. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { firstFreeChord } from "../dist/lib/free-chord.js";

const DEFAULTS = ["Right Alt", "Ctrl+Win+Space", "Esc", "Alt+Shift+Z", "Alt+Shift+X", "Ctrl+Win+Alt", "Ctrl+Win+Shift"];

test("the first Win+Shift letter, skipping the ones Windows takes", () => {
  // A is Windows' (tips), so B comes first.
  assert.equal(firstFreeChord([...DEFAULTS, "Win+Shift+Q", "Win+Shift+W", "Win+Alt+O"]), "Win+Shift+B");
  assert.equal(firstFreeChord(["Win+Shift+B"]), "Win+Shift+D", "C (charms) is skipped too");
});

test("never a letter Windows takes, nor a Murmur shortcut (any case)", () => {
  const used = ["win+shift+b", "Win+Shift+D", "Win+Shift+E", "Win+Shift+F", "Win+Shift+G", "Win+Shift+H", "Win+Shift+I", "Win+Shift+J", "Win+Shift+K", "Win+Shift+L"];
  assert.equal(firstFreeChord(used), "Win+Shift+N", "M (restore windows) skipped");
  for (const l of "ACMRSTV") assert.notEqual(firstFreeChord([]).at(-1), l);
});

test("no default when every free letter is used; blanks are ignored", () => {
  const all = [..."BDEFGHIJKLNOPQUWXYZ"].map((l) => `Win+Shift+${l}`);
  assert.equal(firstFreeChord([...all, null, "", undefined]), "");
  assert.equal(firstFreeChord([null, ""]), "Win+Shift+B");
});
