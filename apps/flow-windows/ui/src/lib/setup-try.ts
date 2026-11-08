// Setup's "Now try it" step (D75): the line under the practice box. The
// step used to show something only after a dictation finished, so a press
// that could not start (no speech model, no server) left it silent while
// the reason went to the pill alone. Pure, so `npm test` runs it on the
// built dist without a page.

/** `setup_readiness`: can a dictation start now (router_adapter::Readiness). */
export interface Readiness {
  ready: boolean;
  engine: string | null;
  message: string | null;
}

/** The pill's state (dictation::OverlayState, `dictation:status`). */
export interface PillState {
  state: string;
  message?: string;
}

/** A backend sentence without its final full stop, to put inside ours. */
const bare = (m: string): string => m.trim().replace(/\.+$/, "");

export interface TryLine {
  text: string;
  bad: boolean;
}

/** Before the user talks: `null` = ready, nothing to say. `downloading`: a
 *  speech model download started in the model step is still running. */
export function readinessLine(r: Readiness, downloading = false): TryLine | null {
  if (r.ready) return null;
  if (downloading) {
    return { text: "The speech model is still downloading. Hold the shortcut once it's done.", bad: false };
  }
  return {
    text: `Dictation can't work yet. ${r.message ?? "No speech engine is set up."} You can finish setup now and fix it later in Settings → Speech to text.`,
    bad: true,
  };
}

/** A failed readiness call (e.g. the app is still starting). */
export function readinessErrorLine(e: unknown): TryLine {
  return { text: `Couldn't check whether dictation is ready: ${String(e)}`, bad: true };
}

/** While and after the user holds the shortcut. `null` = keep the current line
 *  (resting, or a finished dictation, which `dev:route` / history report). */
export function pillLine(s: PillState): TryLine | null {
  switch (s.state) {
    case "active_ptt":
      return { text: "Listening… let go when you're done.", bad: false };
    case "active_locked":
      return { text: "Listening…", bad: false };
    case "processing":
      return { text: "Turning your words into text…", bad: false };
    case "error":
      return {
        text: `That didn't work: ${bare(s.message ?? "unknown error")}. Try again, or check Settings → Speech to text after setup.`,
        bad: true,
      };
    case "notice":
      return { text: `${bare(s.message ?? "Nothing came through")}. Hold the shortcut, speak, then let go.`, bad: true };
    default:
      return null;
  }
}
