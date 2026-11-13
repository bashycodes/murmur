// Re-running setup (Settings → General → Run setup): what the saved settings
// say for each choice setup can change, and the one line before Finish that
// says what Finish leaves changed ("Cleanup: Your server → Rules only") with
// Keep / Change (UX review on 1.3.0: a re-run used to replace a working
// setup silently, D95). Choices are compared as saved (paths, addresses),
// never by their display names; names are only for the line. Pure, so
// `npm test` runs it on the built dist without a page.

import type { SetupCleanup } from "./setup-cleanup.js";
import { cloudShortName } from "./provider-names.js";
import { ENGINE_LABEL, type SetupEngine } from "./setup-engine.js";
import type { CloudProvider } from "./types.js";

/** The speech model on this PC, as the prefs save it. */
export interface SpeechModel {
  asr_engine: string;
  model_path: string | null;
  parakeet_model_dir: string | null;
}

/** Setup's choices as the saved settings have them. */
export interface SetupConfig {
  /** Where speech runs: the first engine the saved settings would use (null: none set up). */
  engine: SetupEngine | null;
  /** Your server's address, trimmed ("" = none). */
  host: string;
  /** How many times a server token was saved since setup opened (the token itself is never read). */
  tokenSaves: number;
  /** null: this build has no speech model on this PC. */
  model: SpeechModel | null;
  /** Where downloads come from (`model_source`). */
  source: string;
  cleanup: SetupCleanup;
  /** The cleanup model file on this PC (null: none / the default). */
  llmPath: string | null;
  /** Push-to-talk shortcut. */
  ptt: string;
  /** Microphone (null or "": the system default). */
  mic: string | null;
}

export type SetupKey = "engine" | "host" | "token" | "model" | "source" | "cleanup" | "llmPath" | "ptt" | "mic";

export interface SetupChange {
  key: SetupKey;
  what: string;
  from: string;
  to: string;
}

/** How a choice reads in the line; `modelName` names a speech model for display only. */
export interface Labels {
  modelName: (m: SpeechModel) => string;
  /** The saved provider: "Cloud" is OpenRouter only, the other is "Your own AI server" (D96). */
  provider?: CloudProvider;
}

const WHAT: Record<SetupKey, string> = {
  engine: "Speech",
  host: "Server address",
  token: "Server token",
  model: "Speech model",
  source: "Download from",
  cleanup: "Cleanup",
  llmPath: "Cleanup model",
  ptt: "Shortcut",
  mic: "Microphone",
};

const CLEANUP: Record<SetupCleanup, string> = { rules: "Rules only", pc: "On this PC", server: "Your server", cloud: "Cloud" };
const SOURCE: Record<string, string> = { server: "Your server", huggingface: "Hugging Face" };

const fileName = (p: string): string => p.split(/[\\/]/).pop() || p;
const sameModel = (a: SpeechModel | null, b: SpeechModel | null): boolean =>
  a === b ||
  (!!a &&
    !!b &&
    a.asr_engine === b.asr_engine &&
    // Only the engine's own file matters: Parakeet keeps a Whisper path set, and the other way round.
    (a.asr_engine === "parakeet" ? (a.parakeet_model_dir ?? "") === (b.parakeet_model_dir ?? "") : (a.model_path ?? "") === (b.model_path ?? "")));

/** Each choice: whether two configs differ, and how one reads. */
const CHOICES: Record<SetupKey, { same: (a: SetupConfig, b: SetupConfig) => boolean; label: (c: SetupConfig, l: Labels) => string }> = {
  engine: { same: (a, b) => a.engine === b.engine, label: (c, l) => (c.engine === "cloud" ? cloudShortName(l.provider) : c.engine ? ENGINE_LABEL[c.engine] : "Not set up") },
  host: { same: (a, b) => a.host.trim() === b.host.trim(), label: (c) => c.host.trim() || "None" },
  token: { same: (a, b) => a.tokenSaves === b.tokenSaves, label: (c) => (c.tokenSaves ? "Replaced" : "As it was") },
  model: { same: (a, b) => sameModel(a.model, b.model), label: (c, l) => (c.model ? l.modelName(c.model) : "None") },
  source: { same: (a, b) => a.source === b.source, label: (c) => SOURCE[c.source] ?? c.source },
  cleanup: { same: (a, b) => a.cleanup === b.cleanup, label: (c, l) => (c.cleanup === "cloud" ? cloudShortName(l.provider) : CLEANUP[c.cleanup]) },
  llmPath: { same: (a, b) => (a.llmPath ?? "") === (b.llmPath ?? ""), label: (c) => (c.llmPath ? fileName(c.llmPath) : "None") },
  ptt: { same: (a, b) => a.ptt === b.ptt, label: (c) => c.ptt.replace(/\+/g, " + ") },
  mic: { same: (a, b) => (a.mic || "") === (b.mic || ""), label: (c) => c.mic || "System default" },
};

const ORDER: SetupKey[] = ["engine", "host", "token", "model", "source", "cleanup", "llmPath", "ptt", "mic"];

/** A choice in setup's own words. */
export function choiceLabel(key: SetupKey, c: SetupConfig, l: Labels): string {
  return CHOICES[key].label(c, l);
}

/** What differs between the settings setup opened with and the ones it would finish with, in step order. */
export function setupChanges(before: SetupConfig, after: SetupConfig, l: Labels): SetupChange[] {
  return ORDER.filter((k) => !CHOICES[k].same(before, after)).map((k) => ({
    key: k,
    what: WHAT[k],
    from: choiceLabel(k, before, l),
    to: choiceLabel(k, after, l),
  }));
}

/** The line before Finish. A new token reads "Server token replaced" (its value is never shown). */
export function changesLine(changes: SetupChange[]): string {
  if (!changes.length) return "Nothing changes. Finish keeps your setup as it was.";
  const part = (c: SetupChange): string => (c.key === "token" ? "Server token replaced" : `${c.what}: ${c.from} → ${c.to}`);
  return `Finish changes ${changes.map(part).join("; ")}.`;
}

/** The step where a choice is made (Change goes there). "Download from" is on step 4. */
export const STEP_OF: Record<SetupKey, "engine" | "model" | "cleanup" | "try"> = {
  engine: "engine",
  host: "engine",
  token: "engine",
  model: "model",
  source: "cleanup",
  cleanup: "cleanup",
  llmPath: "cleanup",
  ptt: "try",
  mic: "try",
};

/** The speech routes as the developer form saves them. */
export interface RouteFacts {
  route_order: ("client" | "server" | "cloud")[];
  allow_client: boolean;
  allow_server: boolean;
  host: string;
  /** OpenRouter speech is on. */
  cloudStt: boolean;
}

const ENGINE_OF = { client: "pc", server: "server", cloud: "cloud" } as const;

/** The first engine the saved settings would use: on, and (a server) with an address. */
export function savedEngine(f: RouteFacts): SetupEngine | null {
  const first = f.route_order.find((r) =>
    r === "client" ? f.allow_client : r === "server" ? f.allow_server && !!f.host.trim() : f.cloudStt,
  );
  return first ? ENGINE_OF[first] : null;
}
