// The sidebar's Engines card: which engine turns speech into text and
// which cleans it up right now, and the fallback. Built only from what the
// backend already reports (dev_get, dev_or_get, get_model_status and their
// events); a click opens Settings → Speech to text.

import { cleanupTarget } from "./features/openrouter.js";
import { $ } from "./lib/dom.js";
import { serverCleanupOn } from "./lib/cleanup-order.js";
import { cloudCleanupUsable, cloudLiveLabel, providerModelLabel, providerOf } from "./lib/provider-names.js";
import { invoke, onEvent } from "./lib/ipc.js";
import { prefsStore } from "./lib/prefs-store.js";
import type { CleanupEngine, OrView, Route } from "./lib/types.js";
import { views } from "./lib/views.js";

type Load = { state: string; path?: string };

interface ModelInfo {
  status: Load;
  llm: Load;
}

let model: Load = { state: "loading" };
let llm: Load = { state: "off" };

const DOT: Record<string, string> = {
  ready: "var(--success-500)",
  loading: "var(--warning-400)",
  // Not loaded, loaded when needed (D24).
  idle: "var(--toggle-border)",
  missing: "var(--destructive-500)",
  failed: "var(--destructive-500)",
  plugin_missing: "var(--destructive-500)",
  off: "var(--toggle-border)",
};

function fileStem(path: string | undefined): string {
  const f = (path ?? "").split(/[\\/]/).pop() ?? "";
  return f.replace(/\.(gguf|bin)$/i, "");
}

function routeOn(r: Route): boolean {
  const dv = views.dev;
  if (!dv) return false;
  if (r === "server") return dv.caps.remote && dv.form.allow_server;
  if (r === "client") return dv.caps.local && dv.form.allow_client;
  return dv.caps.cloud && !!views.or?.supported && !!views.or.stt.allow;
}

function routeLabel(r: Route): [string, string] {
  const dv = views.dev;
  if (r === "server") {
    const h = dv?.route.health;
    return [`Your server · ${dv?.form.host || "not set up"}`, !h ? DOT.off : h.ok ? DOT.ready : DOT.failed];
  }
  if (r === "client") {
    const engine = prefsStore.get()?.asr_engine === "parakeet" ? "Parakeet" : "Whisper";
    return [`This PC · ${engine}`, DOT[model.state] ?? DOT.off];
  }
  return [cloudLiveLabel(providerOf(views.or), providerModelLabel(views.or, "stt")), "var(--glow-300)"];
}

const FALLBACK: Record<Route, string> = {
  server: "Falls back to your server",
  client: "Falls back to this PC",
  cloud: "Falls back to the cloud",
};

/** D96: the cloud fallback names your own AI server when that is the provider. */
function fallbackText(r: Route): string {
  return r === "cloud" && providerOf(views.or) === "custom" ? "Falls back to your own AI server" : FALLBACK[r];
}

/**
 * The cloud cleanup engine's label when the router can use it: consent
 * given (`consent` is the backend's `consent_ok()`, bound to the custom
 * server's current host); D52: a custom server with a model, named by its
 * host, never "OpenRouter"; D96: "Your own AI server", never "Cloud".
 */
function cloudCleanup(or: OrView | null): string | null {
  if (!cloudCleanupUsable(or)) return null;
  const p = providerOf(or);
  return cloudLiveLabel(p, p === "custom" ? cleanupTarget(or) : providerModelLabel(or, "cleanup"));
}

function cleanupLabel(): [string, string] {
  const p = prefsStore.get();
  if (!p) return ["", DOT.off];
  if (p.cleanup_level === "none") return [p.spoken_cleanup ? "Formatting rules only" : "Off (verbatim)", DOT.off];
  const or = views.or;
  const dv = views.dev;
  // D36: the first engine of the cleanup order that is on (and, for the
  // server, set up and not known to lack an AI model).
  const engines: CleanupEngine[] = or?.cleanup.engines ?? [
    { route: "local", enabled: true },
    { route: "server", enabled: true },
  ];
  for (const e of engines) {
    const cloud = e.route === "cloud" ? cloudCleanup(or) : null;
    if (cloud) return [cloud, "var(--glow-300)"];
    if (e.route === "local" && e.enabled && p.llm_enabled && dv?.caps.local)
      return [`This PC · ${fileStem(llm.path) || "local model"}`, DOT[llm.state] ?? DOT.off];
    if (e.route === "server" && dv && serverCleanupOn([e], dv.form.host, dv.caps.remote, or?.cleanup.status.server_llm)) {
      const h = dv.route?.health;
      return [`Your server · ${dv.form.host}`, !h ? DOT.off : h.ok ? DOT.ready : DOT.failed];
    }
  }
  return ["Formatting rules only", DOT.off];
}

function render(): void {
  const dv = views.dev;
  const card = $<HTMLElement>("engines_card");
  card.hidden = !dv;
  if (!dv) return;
  const order = dv.form.route_order.filter(routeOn);
  const [stt, sttDot] = order.length ? routeLabel(order[0]) : ["Off: no engine is on", DOT.failed];
  $<HTMLElement>("eng_stt").textContent = stt;
  $<HTMLElement>("eng_stt_dot").style.background = sttDot;
  const [ai, aiDot] = cleanupLabel();
  $<HTMLElement>("eng_ai").textContent = ai;
  $<HTMLElement>("eng_ai_dot").style.background = aiDot;
  const fb = $<HTMLElement>("eng_fallback");
  fb.hidden = order.length < 2;
  fb.textContent = order.length > 1 ? fallbackText(order[1]) : "";
}

export const enginesCard = {
  init(): void {
    views.onDev(render);
    views.onOr(render);
    prefsStore.subscribe(render);
    void onEvent<Load>("model:status", (e) => {
      model = e.payload;
      render();
    });
    void onEvent<Load>("llm:status", (e) => {
      llm = e.payload;
      render();
    });
    void onEvent<unknown>("dev:route", () => setTimeout(render, 0));
  },
  async load(): Promise<void> {
    const m = await invoke<ModelInfo>("get_model_status");
    model = m.status;
    llm = m.llm;
    render();
  },
};
