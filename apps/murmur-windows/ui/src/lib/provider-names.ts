// D96: what the cloud route is called, from the saved provider. "Cloud"
// means OpenRouter only; the OpenAI-compatible path (D52, wire "custom",
// e.g. a LiteLLM proxy) is "Your own AI server"; "Your server" stays the
// Murmur server. Settings → AI provider is where one of the two is picked.
// Also which model and host the provider uses, and whether its cleanup is
// ready. Pure, so `npm test` runs it on the built dist without a page.

import type { CloudProvider, OrView, Purpose } from "./types.js";

/** The sidebar tab and heading of the page that picks the provider. */
export const PROVIDER_PAGE = "AI provider";
export const OWN_AI_SERVER = "Your own AI server";
export const OWN_AI_SERVER_FULL = "Your own AI server (OpenAI-compatible)";
const OWN_IN_SENTENCE = "your own AI server";

const isOwn = (p: CloudProvider | undefined): boolean => p === "custom";

/** The saved provider (one at a time, D78); OpenRouter when unknown. */
export function providerOf(or: OrView | null | undefined): CloudProvider {
  return or?.cleanup.provider ?? "openrouter";
}

/** The provider's name on its own: "OpenRouter" / "Your own AI server (OpenAI-compatible)". */
export function providerName(p: CloudProvider | undefined): string {
  return isOwn(p) ? OWN_AI_SERVER_FULL : "OpenRouter";
}

/** The cloud route's short name (Insights, Meetings, history): "Cloud" / "Your own AI server". */
export function cloudShortName(p: CloudProvider | undefined): string {
  return isOwn(p) ? OWN_AI_SERVER : "Cloud";
}

/** The cloud route's title in an engine list or a card heading. */
export function cloudRouteTitle(p: CloudProvider | undefined): string {
  return isOwn(p) ? OWN_AI_SERVER : "Cloud · OpenRouter";
}

/** The cloud route inside a sentence: "OpenRouter in the cloud" / "your own AI server". */
export function cloudRouteInSentence(p: CloudProvider | undefined): string {
  return isOwn(p) ? OWN_IN_SENTENCE : "OpenRouter in the cloud";
}

/** Where the cloud route did the work: "in the cloud" / "on your own AI server". */
export function cloudWhere(p: CloudProvider | undefined): string {
  return isOwn(p) ? `on ${OWN_IN_SENTENCE}` : "in the cloud";
}

/** The sidebar Engines card's live label: "Cloud · <model>" / "Your own AI server · <host>". */
export function cloudLiveLabel(p: CloudProvider | undefined, detail: string): string {
  const head = cloudShortName(p);
  return detail ? `${head} · ${detail}` : head;
}

/** The own AI server by its host, or by name while no host is set (openrouter.rs `own_server_or`). */
export function ownServerOr(host: string | undefined): string {
  return host || OWN_IN_SENTENCE;
}

/** The model the provider uses for `purpose` (full id; "" when none is set). */
export function providerModel(or: OrView | null | undefined, purpose: Purpose): string {
  if (!or) return "";
  const own = isOwn(providerOf(or));
  if (purpose === "stt") return own ? or.stt.custom_model ?? "" : or.stt.model;
  return own ? or.cleanup.custom.model : or.cleanup.model;
}

/** The same model as a short label: OpenRouter's id without its vendor (`meta-llama/x` -> `x`), your own AI server's as typed. */
export function providerModelLabel(or: OrView | null | undefined, purpose: Purpose): string {
  const id = providerModel(or, purpose);
  return isOwn(providerOf(or)) ? id : id.split("/").pop() ?? id;
}

/**
 * The cloud cleanup route could answer now: on, consent given, and a key
 * (OpenRouter) or a model (your own AI server, whose consent is bound to
 * its current host).
 */
export function cloudCleanupUsable(or: OrView | null | undefined): boolean {
  const c = or?.cleanup;
  if (!or?.supported || !c?.allow) return false;
  return isOwn(c.provider) ? c.custom.consent && c.custom.model !== "" : c.consent && or.key.set;
}

/** Setup's copy for the cloud speech card and cleanup option (D96: never "Cloud" or "online" for a home server). */
export interface SetupCloudCopy {
  sttTitle: string;
  sttDesc: string;
  sttFacts: string[];
  cleanTitle: string;
  cleanDesc: string;
}

export function setupCloudCopy(p: CloudProvider | undefined): SetupCloudCopy {
  if (isOwn(p)) {
    return {
      sttTitle: OWN_AI_SERVER,
      sttDesc: "The OpenAI-compatible server you set up. Audio goes to it.",
      sttFacts: ["Needs a speech model on that server", "Set up in Settings → AI provider", "Nothing to download"],
      cleanTitle: OWN_AI_SERVER,
      cleanDesc: "The OpenAI-compatible server you set up cleans the text. Sends text, never audio.",
    };
  }
  return {
    sttTitle: "Cloud",
    sttDesc: "Best accuracy. Audio is sent online.",
    sttFacts: ["Needs an account key", "Pay per use, a few cents an hour of audio", "Nothing to download"],
    cleanTitle: "Cloud",
    cleanDesc: "A large online AI model. Best quality. Sends text, never audio.",
  };
}
