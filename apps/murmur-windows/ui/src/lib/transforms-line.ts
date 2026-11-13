// D96: the Transforms banner's engine sentence, from the live cleanup order.
// It mirrors how the router picks a transform's engine (murmur_router
// `Router::command_cleaner` / `legacy_plan`, router.rs): the server never
// runs transforms; the cloud route is "wanted" when it is on and listed,
// ready or not (a cloud that can't answer then falls back or fails); with
// the cloud wanted and first, this PC follows only when it is on; with
// this PC first and its model loaded, it alone answers; without the cloud
// wanted, this PC's loaded model answers even with its switch off. Pure,
// tested in tests/transforms-line.test.mjs against the router's cases.

import { cloudRouteInSentence } from "./provider-names.js";
import type { CleanupEngine, CloudProvider, OrView } from "./types.js";

/** What the router knows when a transform runs. */
export interface TransformEngines {
  /** The D36 cleanup order, every engine once, with its switch. */
  engines: CleanupEngine[];
  /** Cloud cleanup is allowed in this build (`cloud_cleanup_on` minus the order check). */
  cloudAllowed: boolean;
  provider: CloudProvider | undefined;
  /** This PC's AI model is on (the shell's `base` cleaner). */
  local: boolean;
  /** Your server cleans dictations (it never runs transforms). */
  server: boolean;
}

/** `allow_cloud_cleanup` in a build with cloud cleanup. */
export function cloudCleanupAllowed(or: OrView | null | undefined): boolean {
  return !!or?.supported && !!or.cleanup.allow;
}

/** The engines a transform runs on, in order (router `command_cleaner`). */
export function transformEngines(t: TransformEngines): ("cloud" | "local")[] {
  // legacy_plan's order: the cleanup order without the server and switched-off engines.
  const order = t.engines.filter((e) => e.route !== "server" && e.enabled).map((e) => e.route as "cloud" | "local");
  const wanted = t.cloudAllowed && order.includes("cloud");
  if (!wanted) return t.local ? ["local"] : [];
  if (order[0] === "cloud") return t.local && order.includes("local") ? ["cloud", "local"] : ["cloud"];
  return t.local ? ["local"] : ["cloud"];
}

/** The banner sentence. */
export function transformsEngineLine(t: TransformEngines): string {
  const names = transformEngines(t).map((r) => (r === "cloud" ? cloudRouteInSentence(t.provider) : "this PC"));
  const serverNote = t.server ? " Your server doesn't run transforms." : "";
  if (names.length === 0) {
    return `Needs an AI model on this PC or ${cloudRouteInSentence(t.provider)}: turn one on under AI cleanup.${serverNote}`;
  }
  const list = names.join(", then ");
  return `Uses ${list}.${serverNote}`;
}
