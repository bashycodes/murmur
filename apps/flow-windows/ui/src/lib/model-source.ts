// "Download from: Your server | Hugging Face" (D79): what setup shows for
// it, whether the server has a model, and setup's Skip / Continue on the
// model step. Where downloads really come from is the backend's call
// (server_library::Origin, seen as the catalogue's from_server); this only
// lays the user's pick over it. Pure, so `npm test` runs it on the built
// dist without a page.

export type ModelSource = "server" | "huggingface";

/** The parts of `server_models` this module reads. */
export interface Listing {
  error?: string;
  models: { catalogue_id?: string; slot?: string }[];
}

/**
 * The server's library holds a usable copy of catalogue model `id` (tagged with
 * its id, pins matching, engine in this build). undefined = not known: not read
 * yet or unreadable, so Download stays offered and its error says why.
 */
export function onServer(id: string, lib: Listing | undefined): boolean | undefined {
  if (!lib || lib.error) return undefined;
  return lib.models.some((m) => m.catalogue_id === id && !!m.slot);
}

export interface SourceState {
  /** What the user picked on this setup run (null = nothing yet). */
  pick: ModelSource | null;
  /** Where downloads come from now (the backend: catalogue rows from_server). */
  effective: ModelSource;
  /** This build talks to servers. */
  remote: boolean;
  /** A server address is saved. */
  serverSet: boolean;
}

export interface SourceView {
  /** The switch position. */
  shown: ModelSource;
  /** "Your server" is offered. */
  serverButton: boolean;
  /** "Your server" picked with none saved: show the server fields here. */
  serverForm: boolean;
  /** Download buttons show: they fetch from what the switch says. */
  downloads: boolean;
  /** The "… download page →" links to Hugging Face show. */
  hfLinks: boolean;
}

/** Setup's "Download from" switch over the backend's source. */
export function sourceView(s: SourceState): SourceView {
  const shown: ModelSource = s.remote ? (s.pick ?? s.effective) : "huggingface";
  return {
    shown,
    serverButton: s.remote,
    serverForm: shown === "server" && !s.serverSet,
    downloads: shown === s.effective,
    hfLinks: shown === "huggingface",
  };
}

/** The line next to the "Download from" switch. */
export function sourceHint(v: SourceView, host: string): string {
  if (v.shown === "huggingface") return "huggingface.co · needs the internet";
  if (v.serverForm) return "Add your server below. No internet needed.";
  return `${host || "Your server"} · no internet needed`;
}

export interface ModelStep {
  /** Speech runs on this PC (setup step 2), so a model is required. */
  needed: boolean;
  /** Speech runs on the user's server: the model is a fallback. */
  serverEngine: boolean;
  /** A model on this PC is ready or loading. */
  ready: boolean;
  /** A download is running. */
  busy: boolean;
}

/**
 * Setup's model step footer: "Skip — use my server only" while speech runs on
 * the server and no model is ready (it goes to the next step, the counter is
 * unchanged); Continue once a model is ready or downloading, or always when the
 * model is optional, else shown disabled.
 */
export function modelStepFoot(s: ModelStep): { skip: boolean; next: "enabled" | "disabled" | "hidden" } {
  const started = s.ready || s.busy;
  if (s.serverEngine && !s.ready) return { skip: true, next: started ? "enabled" : "hidden" };
  return { skip: false, next: started || !s.needed ? "enabled" : "disabled" };
}
