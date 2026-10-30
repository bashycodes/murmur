// "Press the keys" for a shortcut field: the keyboard hook records the
// chord the user holds and lets go (`hotkey_capture_start` in src/app.rs,
// src/hotkeys/capture.rs) and answers with a `hotkey:capture` event. The
// hook, not the page, listens: the webview can't tell Right Alt from AltGr
// (Ctrl+Alt) and never sees Win or keys the hook swallows. Hotkeys don't
// fire while recording; it ends on release, Esc, 10 s, or the window
// losing focus.

import { invoke, onEvent } from "./ipc.js";

interface CaptureEvent {
  state: "done" | "cancelled" | "timeout";
  chord?: string | null;
}

/** "done" = the chord to save; anything else = keep the old value. */
export type CaptureResult = { chord: string } | { cancelled: true; message?: string };

let pending: ((r: CaptureResult) => void) | null = null;
let listening = false;

function finish(r: CaptureResult): void {
  const p = pending;
  pending = null;
  p?.(r);
}

function onBlur(): void {
  if (!pending) return;
  void invoke("hotkey_capture_stop").catch(() => undefined);
  finish({ cancelled: true });
}

/** Record one chord. A recording already running ends as cancelled. */
export async function captureChord(): Promise<CaptureResult> {
  if (!listening) {
    listening = true;
    await onEvent<CaptureEvent>("hotkey:capture", (e) => {
      const { state, chord } = e.payload;
      if (state === "done" && chord) finish({ chord });
      else finish({ cancelled: true, message: state === "timeout" ? "No keys pressed: kept the old shortcut." : undefined });
    });
    window.addEventListener("blur", onBlur);
  }
  finish({ cancelled: true });
  const result = new Promise<CaptureResult>((resolve) => (pending = resolve));
  try {
    await invoke("hotkey_capture_start");
  } catch (e) {
    finish({ cancelled: true, message: String(e) });
  }
  return result;
}

/** Leave recording without a chord (Cancel / Type instead). */
export function stopCapture(): void {
  if (!pending) return;
  void invoke("hotkey_capture_stop").catch(() => undefined);
  finish({ cancelled: true });
}

/** Is a recording running? */
export function capturing(): boolean {
  return pending !== null;
}

export const RECORDING_TEXT = "Press keys… Esc to cancel";
