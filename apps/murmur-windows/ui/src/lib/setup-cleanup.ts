// Setup's "Polish what you say?" step: the "Your server" choice (D36 server
// cleanup), which option carries "Recommended", which one starts selected,
// and the cleanup order a choice saves. Pure, so `npm test` runs it on the
// built dist without a page.

import type { CleanupEngine, CleanupRoute } from "./types.js";

export type SetupCleanup = "rules" | "pc" | "server" | "cloud";

/** What setup knows about the user's server. */
export interface ServerFacts {
  /** This build talks to servers (`caps.remote`). */
  build: boolean;
  /** An address is saved (step 2, `hasServer`). */
  saved: boolean;
  /** The server can clean text (`CleanupStatus.server_llm`; null = not known yet). */
  llm: boolean | null | undefined;
}

/** How the "Your server" option shows: hidden, or disabled with the reason, or usable. */
export interface ServerOption {
  hidden: boolean;
  disabled: boolean;
  /** Under the title instead of the usual description (empty = the usual one). */
  reason: string;
}

export function serverOption(s: ServerFacts): ServerOption {
  if (!s.build) return { hidden: true, disabled: true, reason: "" };
  if (!s.saved) return { hidden: false, disabled: true, reason: "Set up your server in step 2 first (Back), or later in Settings." };
  if (s.llm === false) return { hidden: false, disabled: false, reason: "Your server has no AI cleanup model yet, so it is skipped until one is added there." };
  return { hidden: false, disabled: false, reason: "" };
}

/**
 * The option setup recommends: your server when one is saved and not known to
 * lack a cleanup model (the server's answer is only known once the app has
 * talked to it), else this PC when the build has local cleanup.
 */
export function recommendedCleanup(s: ServerFacts, hasPc: boolean): SetupCleanup | null {
  if (s.build && s.saved && s.llm !== false) return "server";
  if (hasPc) return "pc";
  return null;
}

/** What the saved settings say about cleanup, for the option selected when the step opens. */
export interface SavedCleanup {
  cloudAllow: boolean;
  /** The cleanup level is not "none" (Rules only / Verbatim). */
  aiLevel: boolean;
  /** This PC's switch (the `llm_enabled` pref). */
  llmEnabled: boolean;
  /** The D36 cleanup order. */
  engines: CleanupEngine[];
}

/**
 * The option selected when the step opens: Cloud when it is on, Rules when
 * the level has no AI, else whichever of this PC and your server comes first
 * in the cleanup order and is on (the server only when one is saved).
 */
export function initialCleanup(saved: SavedCleanup, serverSaved: boolean): SetupCleanup {
  if (saved.cloudAllow) return "cloud";
  if (!saved.aiLevel) return "rules";
  for (const e of saved.engines) {
    if (e.route === "local" && saved.llmEnabled) return "pc";
    if (e.route === "server" && e.enabled && serverSaved) return "server";
  }
  return "rules";
}

/** The order with `route` switched on or off, the rest unchanged (Settings' switch). */
export function withEngine(list: CleanupEngine[], route: CleanupRoute, enabled: boolean): CleanupEngine[] {
  return list.map((e) => (e.route === route ? { ...e, enabled } : { ...e }));
}

/**
 * The cleanup order a setup choice saves, or null when it stays as it is.
 * Your server: its switch on and first, so it cleans before this PC (which
 * stays a backup when it is on). This PC: ahead of your server again if the
 * server was first. Rules and Cloud keep the order (Cloud saves its own).
 */
export function enginesFor(choice: SetupCleanup, list: CleanupEngine[]): CleanupEngine[] | null {
  const first = (r: CleanupRoute, on?: boolean): CleanupEngine[] => {
    const e = list.find((x) => x.route === r) ?? { route: r, enabled: true };
    return [{ ...e, enabled: on ?? e.enabled }, ...list.filter((x) => x.route !== r).map((x) => ({ ...x }))];
  };
  if (choice === "server") {
    const next = first("server", true);
    return same(next, list) ? null : next;
  }
  if (choice === "pc") {
    const li = list.findIndex((e) => e.route === "local");
    const si = list.findIndex((e) => e.route === "server");
    if (li < 0 || si < 0 || li < si) return null;
    const next = [...list.map((e) => ({ ...e }))];
    const [local] = next.splice(li, 1);
    next.splice(si, 0, local);
    return next;
  }
  return null;
}

function same(a: CleanupEngine[], b: CleanupEngine[]): boolean {
  return a.length === b.length && a.every((e, i) => e.route === b[i].route && e.enabled === b[i].enabled);
}
