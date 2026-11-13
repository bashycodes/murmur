// Re-running setup (Settings → General → Run setup): what the saved settings
// say for each step's choice, and the one line before Finish that says what
// Finish leaves changed ("Cleanup: Your server → Rules only") with Keep /
// Change (UX review on 1.3.0: a re-run used to replace a working setup
// silently, D95). Pure, so `npm test` runs it on the built dist without a
// page.

import type { SetupCleanup } from "./setup-cleanup.js";
import type { SetupEngine } from "./setup-engine.js";

/** Setup's choices as the saved settings have them. */
export interface SetupConfig {
  /** Where speech runs: the first engine the saved settings would use (null: none set up). */
  engine: SetupEngine | null;
  /** The speech model on this PC, by name (null: this build has none). */
  model: string | null;
  cleanup: SetupCleanup;
  /** Push-to-talk shortcut. */
  ptt: string;
  /** Microphone (null: the system default). */
  mic: string | null;
}

export type SetupKey = keyof SetupConfig;

export interface SetupChange {
  key: SetupKey;
  what: string;
  from: string;
  to: string;
}

const WHAT: Record<SetupKey, string> = {
  engine: "Speech",
  model: "Speech model",
  cleanup: "Cleanup",
  ptt: "Shortcut",
  mic: "Microphone",
};

const ENGINE: Record<SetupEngine, string> = { pc: "This PC", server: "Your server", cloud: "Cloud" };
const CLEANUP: Record<SetupCleanup, string> = { rules: "Rules only", pc: "On this PC", server: "Your server", cloud: "Cloud" };

/** A choice in setup's own words. */
export function choiceLabel(key: SetupKey, c: SetupConfig): string {
  switch (key) {
    case "engine":
      return c.engine ? ENGINE[c.engine] : "Not set up";
    case "model":
      return c.model ?? "None";
    case "cleanup":
      return CLEANUP[c.cleanup];
    case "ptt":
      return c.ptt.replace(/\+/g, " + ");
    case "mic":
      return c.mic || "System default";
  }
}

const ORDER: SetupKey[] = ["engine", "model", "cleanup", "ptt", "mic"];

/** What differs between the settings setup opened with and the ones it would finish with, in step order. */
export function setupChanges(before: SetupConfig, after: SetupConfig): SetupChange[] {
  return ORDER.filter((k) => choiceLabel(k, before) !== choiceLabel(k, after)).map((k) => ({
    key: k,
    what: WHAT[k],
    from: choiceLabel(k, before),
    to: choiceLabel(k, after),
  }));
}

/** The line before Finish. */
export function changesLine(changes: SetupChange[]): string {
  if (!changes.length) return "Nothing changes. Finish keeps your setup as it was.";
  return `Finish changes ${changes.map((c) => `${c.what}: ${c.from} → ${c.to}`).join("; ")}.`;
}

/** The step where a choice is made (Change goes there). */
export function stepOf(key: SetupKey): "engine" | "model" | "cleanup" | "try" {
  return key === "engine" ? "engine" : key === "model" ? "model" : key === "cleanup" ? "cleanup" : "try";
}

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
