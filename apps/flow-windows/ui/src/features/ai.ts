// AI cleanup: the local LLM, the cleanup level and (D28) which engine
// cleans the text and in what order, on the AI cleanup settings page
// (design: SettingsCleanup.dc.html). The OpenRouter switch and the order
// select belong to features/openrouter.ts; the ranked list here only
// moves rows and changes that select.

import { $ } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import type { Feature } from "../lib/router.js";
import type { CleanupRoute } from "../lib/types.js";
import { views } from "../lib/views.js";

type LlmStatus =
  | { state: "off" }
  | { state: "loading"; path: string }
  | { state: "ready"; path: string; plugin: string; load_ms: number }
  | { state: "missing"; path: string }
  | { state: "plugin_missing"; path: string; message: string }
  | { state: "failed"; path: string; message: string };

interface ModelInfo {
  llm: LlmStatus;
  default_llm_path: string;
  llm_plugin: string;
}

/** What each cleanup level does (D27), with the design's example of what is pasted. */
const LEVELS: Record<string, [string, string, string]> = {
  verbatim: [
    "Verbatim",
    "Exactly what you said. No punctuation or fixes.",
    "um so the meeting is on tuesday no wait thursday at 3 and uh bring the the slides",
  ],
  none: [
    "Rules only",
    'Punctuation, fillers, numbers and "scratch that" are handled by fixed rules on this PC. No AI model runs.',
    "So the meeting is on Thursday at 3 and bring the slides.",
  ],
  light: [
    "Light",
    "Removes fillers and fixes grammar and obvious slips. Keeps your wording.",
    "So the meeting is on Thursday at 3, and bring the slides.",
  ],
  medium: [
    "Medium",
    "Also smooths run-on sentences and corrections for clarity.",
    "The meeting is on Thursday at 3. Bring the slides.",
  ],
  high: [
    "High",
    "Also tightens wording and structure. Best for email and docs.",
    "Meeting: Thursday at 3 p.m. Please bring the slides.",
  ],
};

let llm: LlmStatus = { state: "off" };

function showLevel(): void {
  const level = $<HTMLSelectElement>("cleanup_level").value;
  for (const b of document.querySelectorAll<HTMLButtonElement>("#level_seg [data-level]")) {
    const on = b.dataset.level === level;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-checked", String(on));
  }
  const [title, desc, sample] = LEVELS[level] ?? ["", "", ""];
  $<HTMLElement>("level_title").textContent = title;
  $<HTMLElement>("level_desc").textContent = desc;
  $<HTMLElement>("level_sample").textContent = sample;
}

function llmText(st: LlmStatus): string {
  switch (st.state) {
    case "off":
      return "Off: the formatting rules only.";
    case "loading":
      return `Loading ${st.path}… (can take a minute; dictation works meanwhile, without cleanup)`;
    case "ready":
      return `${fileStem(st.path)}. Private; loaded in ${(st.load_ms / 1000).toFixed(1)} s.`;
    case "missing":
      return `Model file not found: ${st.path}. Dictation keeps working without cleanup.`;
    case "plugin_missing":
      return `The cleanup engine is missing next to the app: ${st.message}`;
    case "failed":
      return `Model failed to load (${st.path}): ${st.message}`;
  }
}

function fileStem(path: string): string {
  return (path.split(/[\\/]/).pop() ?? path).replace(/\.gguf$/i, "");
}

function showLlm(st: LlmStatus): void {
  llm = st;
  const e = $<HTMLElement>("aieng_desc_local");
  e.textContent = llmText(st);
  e.classList.toggle("err", st.state === "missing" || st.state === "plugin_missing" || st.state === "failed");
  renderEngines();
}

type Kind = "success" | "warning" | "destructive" | "light";

function setBadge(r: CleanupRoute, text: string, kind: Kind): void {
  const b = $<HTMLElement>(`aieng_badge_${r}`);
  b.textContent = text;
  b.className = `fl-badge fl-badge-${kind}`;
}

/** The ranked cleanup engines: OpenRouter and this PC, in the saved order. */
function renderEngines(): void {
  const v = views.or;
  const cloudShown = !!v?.supported;
  const localShown = !!views.dev?.caps.local;
  const order: CleanupRoute[] = v?.cleanup.order[0] === "local" ? ["local", "cloud"] : ["cloud", "local"];
  const shown = order.filter((r) => (r === "cloud" ? cloudShown : localShown));
  const list = $<HTMLElement>("ai_routes");
  const foot = list.querySelector(".lw-foot-row");
  for (const r of order) list.insertBefore($<HTMLElement>(`aieng_row_${r}`), foot);
  shown.forEach((r, i) => {
    const row = $<HTMLElement>(`aieng_row_${r}`);
    (row.querySelector(".lw-rank") as HTMLElement).textContent = String(i + 1);
    (row.querySelector("[data-move=up]") as HTMLButtonElement).disabled = i === 0;
    (row.querySelector("[data-move=down]") as HTMLButtonElement).disabled = i === shown.length - 1;
    (row.querySelector(".lw-move") as HTMLElement).hidden = shown.length < 2;
  });

  const cloudOn = !!v?.cleanup.allow;
  $<HTMLElement>("aieng_row_cloud").classList.toggle("is-off", !cloudOn);
  $<HTMLElement>("aieng_desc_cloud").textContent = cloudOn
    ? `Sends the transcript as text to openrouter.ai · ${(v?.cleanup.model ?? "").split("/").pop()}.`
    : "Sends the transcript as text to openrouter.ai. Asks first.";
  const st = v?.cleanup.status;
  if (!cloudOn) setBadge("cloud", "Off", "light");
  else if (st?.blocked) setBadge("cloud", "Not set up", "warning");
  else if (st?.circuit?.open) setBadge("cloud", "Paused", "warning");
  else setBadge("cloud", "On", "success");

  const localOn = $<HTMLInputElement>("llm_enabled").checked;
  $<HTMLElement>("aieng_row_local").classList.toggle("is-off", !localOn);
  if (!localOn) setBadge("local", "Off", "light");
  else if (llm.state === "ready") setBadge("local", "Ready", "success");
  else if (llm.state === "loading" || llm.state === "off") setBadge("local", "Loading", "light");
  else if (llm.state === "plugin_missing") setBadge("local", "Not installed", "destructive");
  else if (llm.state === "missing") setBadge("local", "Model missing", "destructive");
  else setBadge("local", "Failed", "destructive");
}

/** Move a cleanup engine: the order select is features/openrouter.ts's, which saves it. */
function moveEngine(r: CleanupRoute, dir: -1 | 1): void {
  const sel = $<HTMLSelectElement>("or_cleanup_order");
  const cloudFirst = sel.value !== "local,cloud";
  const cloudUp = (r === "cloud" && dir === -1) || (r === "local" && dir === 1);
  if (cloudUp === cloudFirst) return;
  sel.value = cloudUp ? "cloud,local" : "local,cloud";
  sel.dispatchEvent(new Event("change"));
}

/** D27: cleanup "None" = No AI (formatting still runs); Verbatim = Auto-format off too. */
function cleanupChoice(p: Prefs): string {
  return p.cleanup_level === "none" && !p.spoken_cleanup ? "verbatim" : p.cleanup_level;
}

/** Which engine cleans the text (local LLM toggle + OrView). */
function showAiEngine(): void {
  const v = views.or;
  const level = $<HTMLSelectElement>("cleanup_level").value;
  const local = $<HTMLInputElement>("llm_enabled").checked && !!views.dev?.caps.local;
  const cloud = !!v?.supported && v.cleanup.allow && v.cleanup.order.includes("cloud");
  const or = `OpenRouter (${v?.cleanup.model ?? ""})`;
  let text: string;
  if (level === "none" || level === "verbatim")
    text = `no AI (the cleanup level is ${level === "none" ? "Rules only" : "Verbatim"})`;
  else if (cloud && local)
    text =
      v?.cleanup.order[0] === "local"
        ? `This PC · ${or} only when no local model is loaded`
        : `${or}, then this PC`;
  else if (cloud) text = or;
  else if (local) text = "This PC";
  else text = "the formatting rules only";
  $<HTMLElement>("ai_engine_line").textContent = text;
}

function fill(p: Prefs): void {
  $<HTMLInputElement>("llm_enabled").checked = Boolean(p.llm_enabled);
  $<HTMLSelectElement>("cleanup_level").value = cleanupChoice(p);
  showLevel();
  setField("llm_model_path", p.llm_model_path ?? "");
  setField("llm_lib_path", p.llm_lib_path ?? "");
  ensureOption("llm_timeout_ms", String(p.llm_timeout_ms), `${p.llm_timeout_ms / 1000} seconds`);
  renderEngines();
}

/** Select `value` in a preset list, adding it as a choice when it is not a preset. */
function ensureOption(id: string, value: string, label: string): void {
  const sel = $<HTMLSelectElement>(id);
  if (![...sel.options].some((o) => o.value === value)) {
    const after = [...sel.options].find((o) => Number(o.value) > Number(value));
    sel.add(new Option(label, value), after ?? null);
  }
  sel.value = value;
}

/** Set a field unless the user is typing in it. */
function setField(id: string, v: string): void {
  const i = $<HTMLInputElement>(id);
  if (document.activeElement !== i) i.value = v;
}

/** The prefs fields this module owns. Runs after general's collect,
 *  so Verbatim can switch Auto-format off. */
export function collectAi(): Partial<Prefs> {
  const next: Partial<Prefs> = {};
  next.llm_enabled = $<HTMLInputElement>("llm_enabled").checked;
  const level = $<HTMLSelectElement>("cleanup_level").value;
  next.cleanup_level = level === "verbatim" ? "none" : level;
  if (level === "verbatim") next.spoken_cleanup = false;
  // Raw mode only ever skipped the LLM; "No AI" says that now (D27).
  next.raw_mode = false;
  next.llm_model_path = $<HTMLInputElement>("llm_model_path").value.trim() || null;
  next.llm_lib_path = $<HTMLInputElement>("llm_lib_path").value.trim() || null;
  next.llm_timeout_ms = Math.min(
    30000,
    Math.max(200, parseInt($<HTMLInputElement>("llm_timeout_ms").value, 10) || 3000),
  );
  return next;
}

export const ai: Feature = {
  init(): void {
    $<HTMLInputElement>("llm_enabled").addEventListener("change", showAiEngine);
    $<HTMLInputElement>("llm_enabled").addEventListener("change", renderEngines);
    for (const b of document.querySelectorAll<HTMLButtonElement>("#ai_routes [data-move]")) {
      const r = (b.closest("[data-cleanup]") as HTMLElement).dataset.cleanup as CleanupRoute;
      b.addEventListener("click", () => moveEngine(r, b.dataset.move === "up" ? -1 : 1));
    }
    const level = $<HTMLSelectElement>("cleanup_level");
    for (const b of document.querySelectorAll<HTMLButtonElement>("#level_seg [data-level]")) {
      b.addEventListener("click", () => {
        if (level.value === b.dataset.level) return;
        level.value = b.dataset.level ?? "none";
        // Bubbles to the shell, which saves the prefs.
        level.dispatchEvent(new Event("change", { bubbles: true }));
      });
    }
    level.addEventListener("change", showLevel);
    level.addEventListener("change", showAiEngine);
    // Verbatim and the Auto-format switch are the same switch (D27): keep them in step.
    level.addEventListener("change", () => {
      const fmt = $<HTMLInputElement>("spoken_cleanup");
      if (level.value === "verbatim") fmt.checked = false;
      else if (!fmt.checked && level.value === "none") fmt.checked = true;
    });
    $<HTMLInputElement>("spoken_cleanup").addEventListener("change", () => {
      const fmt = $<HTMLInputElement>("spoken_cleanup");
      if (!fmt.checked && level.value === "none") level.value = "verbatim";
      else if (fmt.checked && level.value === "verbatim") level.value = "none";
      showLevel();
    });
    views.onOr(() => {
      showAiEngine();
      renderEngines();
    });
    views.onDev(() => {
      showAiEngine();
      renderEngines();
    });
    prefsStore.subscribe((p) => {
      fill(p);
      showAiEngine();
    });
    void onEvent<LlmStatus>("llm:status", (e) => showLlm(e.payload));
  },
  async load(): Promise<void> {
    const model = await invoke<ModelInfo>("get_model_status");
    showLlm(model.llm);
    $<HTMLElement>("llm_plugin").textContent = model.llm_plugin;
    $<HTMLInputElement>("llm_model_path").placeholder = model.default_llm_path;
    const p = prefsStore.get();
    if (p) fill(p);
  },
};
