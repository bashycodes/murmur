// Ctrl+, opens Settings from the Murmur window: a key the page handles
// itself (window-local), not a global hotkey. Runs on the built dist:
// `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { isOpenSettings, shouldOpenSettings } from "../dist/lib/app-keys.js";

const press = (over = {}) => ({
  key: ",",
  code: "Comma",
  ctrlKey: true,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...over,
});
const idle = { settingsOpen: false, onboardingOpen: false, dialogOpen: false };

test("Ctrl+, matches", () => {
  assert.equal(isOpenSettings(press()), true);
});

test("the comma key matches by its place on another layout", () => {
  assert.equal(isOpenSettings(press({ key: "ת" })), true);
});

test("a comma on another key matches by the character", () => {
  assert.equal(isOpenSettings(press({ code: "KeyM" })), true);
});

test("no Ctrl, or an extra modifier, is not the shortcut", () => {
  assert.equal(isOpenSettings(press({ ctrlKey: false })), false);
  assert.equal(isOpenSettings(press({ shiftKey: true })), false);
  assert.equal(isOpenSettings(press({ altKey: true })), false);
  assert.equal(isOpenSettings(press({ metaKey: true })), false);
  assert.equal(isOpenSettings(press({ key: ".", code: "Period" })), false);
});

test("opens Settings from the main window", () => {
  assert.equal(shouldOpenSettings(press(), idle), true);
});

test("does nothing when Settings is already open, or setup or a dialog is over the shell", () => {
  assert.equal(shouldOpenSettings(press(), { ...idle, settingsOpen: true }), false);
  assert.equal(shouldOpenSettings(press(), { ...idle, onboardingOpen: true }), false);
  assert.equal(shouldOpenSettings(press(), { ...idle, dialogOpen: true }), false);
});
