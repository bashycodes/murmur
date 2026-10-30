// Light or dark: the single class .lw-dark on <html> switches every token
// (theme.css). The `theme` pref (Settings → General → Theme) picks
// "light", "dark" or "system"; "system" follows Windows' app mode live
// through prefers-color-scheme. The last choice is remembered in
// localStorage so the window does not flash light before prefs load.

import { prefsStore } from "./prefs-store.js";

export type ThemePref = "system" | "light" | "dark";

const media = window.matchMedia("(prefers-color-scheme: dark)");
const STORE_KEY = "lw-theme";
let pref: ThemePref = "system";

function parse(v: unknown): ThemePref {
  return v === "light" || v === "dark" ? v : "system";
}

function apply(): void {
  const dark = pref === "dark" || (pref === "system" && media.matches);
  document.documentElement.classList.toggle("lw-dark", dark);
}

/** Use `t` from now on (and remember it for the next window open). */
export function setTheme(t: unknown): void {
  pref = parse(t);
  try {
    localStorage.setItem(STORE_KEY, pref);
  } catch {
    // Storage blocked: the pref still applies, the next open may flash.
  }
  apply();
}

/** Apply the remembered theme now, then follow the pref and Windows' mode. */
export function followSystemTheme(): void {
  try {
    pref = parse(localStorage.getItem(STORE_KEY));
  } catch {
    pref = "system";
  }
  apply();
  media.addEventListener("change", apply);
  prefsStore.subscribe((p) => {
    if (parse(p.theme) !== pref) setTheme(p.theme);
  });
}
