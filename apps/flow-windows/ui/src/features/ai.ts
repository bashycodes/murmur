// AI cleanup: the local LLM, the cleanup level and (D28) which engine
// cleans the text, on the AI cleanup settings page.

import { $, setValue } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import type { Feature } from "../lib/router.js";
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

/** What each cleanup level does (D27). */
const LEVELS: Record<string, [string, string]> = {
  none: ["No AI", "The formatting rules only: punctuation, spoken commands, numbers. Nothing is rewritten."],
  verbatim: ["Verbatim", "Exactly what you said, with no formatting at all."],
  light: ["Light", "Removes fillers and fixes grammar. Keeps your words."],
  medium: ["Medium", "Also smooths run-on sentences and corrections for clarity."],
  high: ["High", "Rewrites for polish."],
};

function showLevel(): void {
  const level = $<HTMLSelectElement>("cleanup_level").value;
  for (const b of document.querySelectorAll<HTMLButtonElement>("#level_seg [data-level]")) {
    const on = b.dataset.level === level;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-checked", String(on));
  }
  const [title, desc] = LEVELS[level] ?? ["", ""];
  $<HTMLElement>("level_title").textContent = title;
  $<HTMLElement>("level_desc").textContent = desc;
}

function showLlm(st: LlmStatus): void {
  const e = $<HTMLParagraphElement>("llm_status");
  switch (st.state) {
    case "off":
      e.textContent = "Off: deterministic formatting only.";
      e.className = "hint";
      break;
    case "loading":
      e.textContent = `Loading ${st.path}… (can take a minute; dictation works meanwhile, without cleanup)`;
      e.className = "hint";
      break;
    case "ready":
      e.textContent = `Ready: ${st.path} (loaded in ${(st.load_ms / 1000).toFixed(1)} s)`;
      e.className = "hint ok";
      break;
    case "missing":
      e.textContent = `Model file not found: ${st.path}. Dictation keeps working without cleanup.`;
      e.className = "hint err";
      break;
    case "plugin_missing":
      e.textContent = `The cleanup engine is missing next to the app: ${st.message}`;
      e.className = "hint err";
      break;
    case "failed":
      e.textContent = `Model failed to load (${st.path}): ${st.message}`;
      e.className = "hint err";
      break;
  }
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
  if (level === "none" || level === "verbatim") text = "Off: the cleanup level is No AI";
  else if (cloud && local)
    text =
      v?.cleanup.order[0] === "local"
        ? `This PC · ${or} only when no local model is loaded`
        : `${or}, then this PC`;
  else if (cloud) text = or;
  else if (local) text = "This PC";
  else text = "Off: the formatting rules only";
  $<HTMLElement>("ai_engine_line").textContent = text;
}

function fill(p: Prefs): void {
  $<HTMLInputElement>("llm_enabled").checked = Boolean(p.llm_enabled);
  $<HTMLSelectElement>("cleanup_level").value = cleanupChoice(p);
  showLevel();
  setValue("llm_model_path", p.llm_model_path ?? "");
  setValue("llm_lib_path", p.llm_lib_path ?? "");
  setValue("llm_timeout_ms", String(p.llm_timeout_ms));
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
    views.onOr(() => showAiEngine());
    views.onDev(() => showAiEngine());
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
