// Pure rules behind the decluttered Speech to text and AI cleanup pages:
// the This PC model status chip, the "AI engines are skipped" warning, the
// formatting rules that follow the cleanup level (D27) and the one "Give up
// after" that sets every cleanup engine's time limit. No DOM, so node tests
// run them on the built dist.

import type { CloudProvider } from "./types.js";

/** A speech model in the catalogue, as far as the chip needs it. */
export interface PcModelEntry {
  id: string;
  engine: string;
  dest: string;
  language?: string;
  installed: boolean;
  size: number;
}

/** Windows paths compare without case and with either slash. */
const samePath = (a: string, b: string): boolean => a.replace(/\//g, "\\").toLowerCase() === b.replace(/\//g, "\\").toLowerCase();

/** The catalogue model the This PC engine loads: `engine`'s entry at `path` (empty = `defaultPath`). */
export function pcModelEntry<T extends PcModelEntry>(catalog: T[], engine: string, path: string, defaultPath: string): T | undefined {
  const want = path.trim() || defaultPath;
  if (!want) return undefined;
  return catalog.find((c) => c.engine === engine && !c.language && samePath(c.dest, want));
}

export type ChipKind = "success" | "light" | "warning" | "destructive";

/** What the chip next to "This PC model" shows: a badge, or a Download button instead. */
export interface PcModelChip {
  text: string;
  kind: ChipKind;
  download: boolean;
}

/**
 * The status chip of the This PC model. `status` is the backend's model
 * state (`model:status`); it stays "loading" while This PC is off, so an
 * installed model reads "Installed" then, not "Loading".
 */
export function pcModelChip(o: {
  status: string;
  entry?: PcModelEntry;
  canDownload: boolean;
  busy: boolean;
}): PcModelChip {
  const chip = (text: string, kind: ChipKind): PcModelChip => ({ text, kind, download: false });
  if (o.busy) return chip("Downloading…", "light");
  if (o.status === "ready") return chip("Ready", "success");
  if (o.status === "failed") return chip("Failed", "destructive");
  if (o.entry && !o.entry.installed) return o.canDownload ? { text: "", kind: "light", download: true } : chip("Not downloaded", "warning");
  if (o.status === "missing") return chip("Missing", "destructive");
  return o.entry ? chip("Installed", "light") : chip("", "light");
}

/**
 * The chip next to AI cleanup's "This PC model": shown only for a model
 * that is here and not downloading ("In use" when cleanup loads it); a
 * model not here yet gets Download / Import buttons instead (null).
 */
export function llmModelChip(o: { installed: boolean; inUse: boolean; busy: boolean }): PcModelChip | null {
  if (!o.installed || o.busy) return null;
  return o.inUse ? { text: "In use", kind: "success", download: false } : { text: "Installed", kind: "light", download: false };
}

/** Levels that run no AI model (D27): Verbatim and Rules only. */
export const noAiLevel = (level: string): boolean => level === "verbatim" || level === "none";

/** The inline warning under the cleanup order: an AI engine is on but the level runs no AI. */
export function skippedWarning(level: string, levelName: string, aiEngineOn: boolean): string {
  return aiEngineOn && noAiLevel(level) ? `AI engines are skipped at ${levelName}.` : "";
}

/**
 * `spoken_cleanup` after the level changed from `prev` to `next` (D27):
 * Verbatim turns the formatting rules off, Rules only and leaving Verbatim
 * turn them on, and a move between AI levels keeps the Advanced choice.
 */
export function rulesAfterLevel(next: string, prev: string, rules: boolean): boolean {
  if (next === "verbatim") return false;
  if (next === "none" || prev === "verbatim") return true;
  return rules;
}

/** The backend's accepted cloud cleanup limits (murmur-router CLEANUP_TIMEOUT_RANGE_MS, CUSTOM_…). */
const CLOUD_RANGE: Record<CloudProvider, [number, number]> = {
  openrouter: [800, 8000],
  custom: [800, 30000],
};

/** The cloud's time limit for the one "Give up after" value: the same, within what the provider accepts. */
export function cloudTimeout(provider: CloudProvider, ms: number): number {
  const [lo, hi] = CLOUD_RANGE[provider];
  return Math.min(hi, Math.max(lo, ms));
}

/** "2.5 seconds". */
export const seconds = (ms: number): string => `${ms / 1000} seconds`;

/**
 * Under "Give up after" while the cloud engine is on and still has its own
 * limit (saved before the two were merged): says so until a time is picked.
 * `cloudMs` null = the cloud is off or not in this build.
 */
export function timeoutNote(localMs: number, provider: CloudProvider, cloudMs: number | null): string {
  if (cloudMs === null || cloudMs === cloudTimeout(provider, localMs)) return "";
  return `The cloud still waits ${seconds(cloudMs)}; pick a time to use one for all.`;
}
