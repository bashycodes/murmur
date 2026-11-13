// D96: Settings → Data and privacy's "What leaves this PC" rows that depend
// on the engines: the text your server cleans, and the audio and text rows
// of the picked provider, named by it (OpenRouter is "the cloud", the
// OpenAI-compatible one "your own AI server"). Pure, tested in
// tests/privacy-rows.test.mjs.

import { ownServerOr, providerModel, providerOf } from "./provider-names.js";
import type { OrView } from "./types.js";

/** One "What leaves this PC" row. */
export interface LeaveRow {
  title: string;
  desc: string;
  on: boolean;
}

/** "Text to your server": the dictated text your server cleans. */
export function serverTextLeave(on: boolean, host: string, tls: boolean): LeaveRow {
  return {
    title: "Text to your server",
    on,
    desc: on
      ? `For AI cleanup on ${host}${tls ? ", encrypted" : ""}. Your own machine; text goes nowhere else.`
      : "Off: your server doesn't clean text.",
  };
}

/** The audio and text rows of the picked provider. */
export function cloudLeaves(or: OrView): { audio: LeaveRow; text: LeaveRow } {
  const own = providerOf(or) === "custom";
  const where = own ? "your own AI server" : "the cloud";
  const at = own ? ` at ${ownServerOr(or.cleanup.custom.host)}` : "";
  const zdr = !own && or.privacy.zdr ? " Zero-retention providers only." : "";
  const off = "Off. Turning it on asks first.";
  const cleanModel = providerModel(or, "cleanup") || "no model picked yet";
  return {
    audio: {
      title: `Audio to ${where}`,
      on: or.stt.allow,
      desc: or.stt.allow ? `For speech to text${at} with ${providerModel(or, "stt")}.` : off,
    },
    text: {
      title: `Text to ${where}`,
      on: or.cleanup.allow,
      desc: or.cleanup.allow ? `For AI cleanup${at} with ${cleanModel}.${zdr}` : off,
    },
  };
}
