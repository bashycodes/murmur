// Setup's speech-model step (step 3): which steps setup shows, the two rows'
// names, and the state chip each row carries (Installed / Downloading 34% /
// Not installed). Paths, the drop zone and "Download from" live in Settings
// → Speech to text (UX review on 1.3.0). Pure, so `npm test` runs it on the
// built dist without a page.

export type SetupStep = "welcome" | "engine" | "model" | "cleanup" | "try";
export type SpeechRow = "parakeet" | "whisper";

/**
 * The steps setup walks. The speech-model step only when speech runs on this
 * PC: with your server or the cloud doing the listening there is nothing to
 * set up here (a model for this PC can be added later in Settings).
 */
export function setupSteps(hasPc: boolean, engine: "pc" | "server" | "cloud"): SetupStep[] {
  return hasPc && engine === "pc" ? ["welcome", "engine", "model", "cleanup", "try"] : ["welcome", "engine", "cleanup", "try"];
}

/** The speech model's state as `get_model_status` / `model:status` report it. */
export interface LoadState {
  state: string;
  path?: string;
}

/**
 * The model file the prefs point at is on this PC: loaded, loading, failed to
 * load (it is there but broken) or `idle` (there, loaded when needed, D24).
 * Only `missing` says it is not. An `idle` or `loading` with no path is the
 * old "never loaded" report, which says nothing about the file.
 */
export function filePresent(s: LoadState | null | undefined): boolean | undefined {
  if (!s) return undefined;
  if (s.state === "missing") return false;
  if ((s.state === "idle" || s.state === "loading") && !s.path) return undefined;
  return true;
}

/**
 * The row picked when the step opens: the model in use once setup has been
 * finished (a re-run keeps the current config) or when its file is there;
 * on a new install with nothing there yet, the recommended one (the
 * built-in default is Whisper, which a new install never chose).
 */
export function initialModel(active: SpeechRow, onboarded: boolean, present: boolean | undefined): SpeechRow {
  return onboarded || present === true ? active : "parakeet";
}

export interface Chip {
  state: "installed" | "downloading" | "missing";
  text: string;
  /** The `fl-badge-*` colour. */
  kind: "success" | "light" | "warning";
}

/** The row's chip: a download in progress wins, then the file being there. */
export function chip(installed: boolean, downloading: number | null): Chip {
  if (downloading !== null) return { state: "downloading", text: `Downloading ${downloading}%`, kind: "light" };
  if (installed) return { state: "installed", text: "Installed", kind: "success" };
  return { state: "missing", text: "Not installed", kind: "warning" };
}

/** A catalogue speech model, as far as this step reads it. */
export interface CatalogModel {
  id: string;
  title: string;
  engine: string;
  dest: string;
  language?: string;
}

/** The Whisper row: the catalogue model it stands for (null: a file of the user's own) and its name. */
export interface WhisperRow {
  id: string | null;
  title: string;
}

/** "C:\…\ggml-large-v3-turbo.bin" -> "Whisper large-v3-turbo". */
export function whisperName(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? "";
  const stem = file.replace(/\.bin$/i, "").replace(/^ggml-/i, "");
  return stem ? `Whisper ${stem}` : "Whisper";
}

/**
 * Which Whisper model the row shows: the file the prefs point at, so its name
 * always matches the file. No file set (or whisper base's own file): the
 * catalogue's whisper base, which Download fetches. Another catalogue file:
 * that model. Any other file: named after the file, nothing to download.
 */
export function whisperRow(modelPath: string | null | undefined, catalog: CatalogModel[], fallbackId = "whisper-base"): WhisperRow {
  const speech = catalog.filter((c) => c.engine === "whisper" && !c.language);
  const path = (modelPath ?? "").trim();
  const byDest = path ? speech.find((c) => sameFile(c.dest, path)) : speech.find((c) => c.id === fallbackId);
  if (byDest) return { id: byDest.id, title: byDest.title };
  if (!path) return { id: null, title: "Whisper base" };
  return { id: null, title: whisperName(path) };
}

function sameFile(a: string, b: string): boolean {
  return a.replace(/\//g, "\\").toLowerCase() === b.replace(/\//g, "\\").toLowerCase();
}
