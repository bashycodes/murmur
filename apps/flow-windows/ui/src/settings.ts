// Settings window: four tabs over the backend commands in `src/app.rs`.
// * General + AI cleanup edit prefs.json (`get_prefs` / `set_prefs`);
//   field names mirror `src/prefs.rs` (snake_case).
// * History lists the local SQLite transcripts (copy, re-paste, delete,
//   undo AI edit = use the raw ASR text instead of the cleaned one).
// * Dictionary edits the [{word, replacement, isSnippet}] list, shows
//   learn-from-edits suggestions (added only on an explicit click) and
//   imports/exports the shared JSON.
// All user text is inserted with textContent, never innerHTML.

interface DictEntry {
  word: string;
  replacement?: string | null;
  isSnippet?: boolean;
}

interface Prefs {
  shortcuts: { ptt: string; hands_free: string; cancel: string; paste_last: string; copy_last: string };
  microphone: string | null;
  languages: string[];
  enable_sounds: boolean;
  mute_audio_while_dictating: boolean;
  launch_at_login: boolean;
  show_flow_bar_always: boolean;
  use_ax_context: boolean;
  cleanup_level: string;
  raw_mode: boolean;
  styles: { personal: string; work: string; email: string; other: string };
  dictionary: DictEntry[];
  paste_method: string;
  restore_clipboard: boolean;
  max_dictation_minutes: number;
  retention: string;
  model_path: string | null;
  llm_enabled: boolean;
  llm_model_path: string | null;
  llm_lib_path: string | null;
  llm_timeout_ms: number;
  learn_from_edits: boolean;
  [k: string]: unknown;
}

type ModelStatus =
  | { state: "loading"; path: string }
  | { state: "ready"; path: string; load_ms: number }
  | { state: "missing"; path: string }
  | { state: "failed"; path: string; message: string };

type LlmStatus =
  | { state: "off" }
  | { state: "loading"; path: string }
  | { state: "ready"; path: string; plugin: string; load_ms: number }
  | { state: "missing"; path: string }
  | { state: "plugin_missing"; path: string; message: string }
  | { state: "failed"; path: string; message: string };

interface ModelInfo {
  status: ModelStatus;
  default_path: string;
  models_dir: string;
  llm: LlmStatus;
  default_llm_path: string;
  llm_plugin: string;
}

interface HistoryItem {
  id: number;
  ts: number;
  app: string;
  url: string | null;
  raw: string;
  formatted: string | null;
  pasted: string | null;
  edited: string | null;
  duration_ms: number;
  words: number;
  lang: string | null;
  final_text: string;
  ai_edited: boolean;
}

interface Suggestion {
  word: string;
  heard: string;
  app: string;
  ts: number;
  count: number;
}

interface MergeReport {
  entries: DictEntry[];
  added: number;
  updated: number;
  skipped: number;
}

const { invoke } = window.__TAURI__.core;
const onEvent = <T>(name: string, cb: (e: TauriEvent<T>) => void): Promise<() => void> =>
  window.__TAURI__.event.listen<T>(name, cb);

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function button(label: string, onClick: () => void, cls = "small"): HTMLButtonElement {
  const b = el("button", cls, label);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

function setStatus(id: string, text: string, ok: boolean): void {
  const s = $<HTMLElement>(id);
  s.textContent = text;
  s.className = ok ? "ok" : "err";
}

const BOOL_FIELDS = [
  "enable_sounds",
  "mute_audio_while_dictating",
  "launch_at_login",
  "show_flow_bar_always",
  "use_ax_context",
  "raw_mode",
  "restore_clipboard",
  "live_preview",
  "llm_enabled",
  "learn_from_edits",
] as const;

const STYLES: [string, string][] = [
  ["formal", "Formal — capitals, full punctuation"],
  ["casual", "Casual — capitals, lighter punctuation"],
  ["very_casual", "Very casual — no caps, less punctuation"],
  ["excited", "Excited — more exclamations!"],
];
const BUCKETS = ["personal", "work", "email", "other"] as const;

let current: Prefs | null = null;

// ------------------------------------------------------------------ tabs

function showTab(name: string): void {
  for (const b of document.querySelectorAll<HTMLButtonElement>("nav.tabs button")) {
    b.setAttribute("aria-selected", String(b.dataset.tab === name));
  }
  for (const p of document.querySelectorAll<HTMLElement>("[data-panel]")) {
    p.hidden = p.dataset.panel !== name;
  }
  $<HTMLFormElement>("form").hidden = name !== "general" && name !== "ai";
  if (name === "history") void loadHistory();
  if (name === "dictionary") void loadSuggestions();
}

// ------------------------------------------------------------------ status lines

function showModel(st: ModelStatus): void {
  const e = $<HTMLParagraphElement>("model_status");
  switch (st.state) {
    case "loading":
      e.textContent = `Loading ${st.path}…`;
      e.className = "hint";
      break;
    case "ready":
      e.textContent = `Ready: ${st.path} (loaded in ${st.load_ms} ms)`;
      e.className = "hint ok";
      break;
    case "missing":
      e.textContent = `Model file not found: ${st.path}. Dictation is unavailable until it exists.`;
      e.className = "hint err";
      break;
    case "failed":
      e.textContent = `Model failed to load (${st.path}): ${st.message}`;
      e.className = "hint err";
      break;
  }
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

// ------------------------------------------------------------------ general + AI form

function fillForm(prefs: Prefs): void {
  $<HTMLInputElement>("model_path").value = prefs.model_path ?? "";
  $<HTMLInputElement>("ptt").value = prefs.shortcuts.ptt;
  $<HTMLInputElement>("hands_free").value = prefs.shortcuts.hands_free;
  $<HTMLInputElement>("cancel").value = prefs.shortcuts.cancel;
  $<HTMLInputElement>("paste_last").value = prefs.shortcuts.paste_last;
  $<HTMLInputElement>("copy_last").value = prefs.shortcuts.copy_last;
  $<HTMLSelectElement>("microphone").value = prefs.microphone ?? "";
  for (const f of BOOL_FIELDS) $<HTMLInputElement>(f).checked = Boolean(prefs[f]);
  $<HTMLSelectElement>("cleanup_level").value = prefs.cleanup_level;
  $<HTMLSelectElement>("paste_method").value = prefs.paste_method;
  $<HTMLInputElement>("languages").value = prefs.languages.join(", ");
  $<HTMLInputElement>("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
  $<HTMLInputElement>("llm_model_path").value = prefs.llm_model_path ?? "";
  $<HTMLInputElement>("llm_lib_path").value = prefs.llm_lib_path ?? "";
  $<HTMLInputElement>("llm_timeout_ms").value = String(prefs.llm_timeout_ms);
  for (const b of BUCKETS) {
    const v = prefs.styles[b].toLowerCase().replace(/[ -]/g, "_");
    $<HTMLSelectElement>(`style_${b}`).value = STYLES.some(([k]) => k === v) ? v : "formal";
  }
  $<HTMLSelectElement>("retention").value = prefs.retention;
}

function readForm(base: Prefs): Prefs {
  const next: Prefs = { ...base };
  next.shortcuts = {
    ptt: $<HTMLInputElement>("ptt").value.trim(),
    hands_free: $<HTMLInputElement>("hands_free").value.trim(),
    cancel: $<HTMLInputElement>("cancel").value.trim(),
    paste_last: $<HTMLInputElement>("paste_last").value.trim(),
    copy_last: $<HTMLInputElement>("copy_last").value.trim(),
  };
  next.model_path = $<HTMLInputElement>("model_path").value.trim() || null;
  next.microphone = $<HTMLSelectElement>("microphone").value || null;
  for (const f of BOOL_FIELDS) next[f] = $<HTMLInputElement>(f).checked;
  next.cleanup_level = $<HTMLSelectElement>("cleanup_level").value;
  next.paste_method = $<HTMLSelectElement>("paste_method").value;
  next.languages = $<HTMLInputElement>("languages")
    .value.split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  next.max_dictation_minutes = Math.max(
    1,
    parseInt($<HTMLInputElement>("max_dictation_minutes").value, 10) || 20,
  );
  next.llm_model_path = $<HTMLInputElement>("llm_model_path").value.trim() || null;
  next.llm_lib_path = $<HTMLInputElement>("llm_lib_path").value.trim() || null;
  next.llm_timeout_ms = Math.min(
    30000,
    Math.max(200, parseInt($<HTMLInputElement>("llm_timeout_ms").value, 10) || 1500),
  );
  next.styles = {
    personal: $<HTMLSelectElement>("style_personal").value,
    work: $<HTMLSelectElement>("style_work").value,
    email: $<HTMLSelectElement>("style_email").value,
    other: $<HTMLSelectElement>("style_other").value,
  };
  return next;
}

/** Save `next` and make it the base for every tab. */
async function savePrefs(next: Prefs): Promise<void> {
  await invoke("set_prefs", { prefs: next });
  current = next;
}

async function saveForm(ev: Event): Promise<void> {
  ev.preventDefault();
  if (!current) return;
  try {
    await savePrefs(readForm(current));
    setStatus("status", "Saved", true);
  } catch (e) {
    setStatus("status", String(e), false);
  }
}

// ------------------------------------------------------------------ history

/** Word-level LCS diff: [kind, word] with kind "=", "-" (raw only), "+" (final only). */
function wordDiff(a: string, b: string): [string, string][] {
  const x = a.split(/\s+/).filter(Boolean);
  const y = b.split(/\s+/).filter(Boolean);
  const n = x.length;
  const m = y.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: [string, string][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      out.push(["=", x[i]]);
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) out.push(["-", x[i++]]);
    else out.push(["+", y[j++]]);
  }
  while (i < n) out.push(["-", x[i++]]);
  while (j < m) out.push(["+", y[j++]]);
  return out;
}

function diffView(raw: string, final: string): HTMLElement {
  const p = el("p", "diff");
  for (const [kind, w] of wordDiff(raw, final)) {
    const tag = kind === "-" ? "del" : kind === "+" ? "ins" : "span";
    p.append(el(tag, undefined, w), " ");
  }
  return p;
}

function when(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

async function act(what: Promise<unknown>, okText: string): Promise<void> {
  try {
    await what;
    setStatus("hist_status", okText, true);
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

function historyRow(h: HistoryItem): HTMLLIElement {
  const li = el("li");
  const meta = el("div", "meta");
  meta.append(el("span", undefined, when(h.ts)));
  if (h.app) meta.append(el("span", undefined, h.app.replace(/\.exe$/i, "")));
  meta.append(el("span", undefined, `${h.words} words`));
  if (h.ai_edited) meta.append(el("span", "chip", "AI edited"));
  if (h.edited) meta.append(el("span", "chip", "you corrected it"));
  if (h.pasted === null) meta.append(el("span", "chip err", "not pasted"));
  li.append(meta, el("p", "text", h.final_text));

  const actions = el("div", "actions");
  actions.append(
    button("Copy", () => void act(invoke("history_copy", { id: h.id, raw: false }), "Copied")),
    button("Re-paste", () => void act(invoke("history_repaste", { id: h.id, raw: false }), "Pasted")),
  );
  if (h.ai_edited) {
    const undo = el("div", "undo");
    undo.hidden = true;
    undo.append(
      el("p", "hint", "What you said (struck = removed by AI, underlined = added):"),
      diffView(h.raw, h.final_text),
      button("Re-paste original", () =>
        void act(invoke("history_repaste", { id: h.id, raw: true }), "Pasted the original"),
      ),
      button("Copy original", () =>
        void act(invoke("history_copy", { id: h.id, raw: true }), "Copied the original"),
      ),
    );
    actions.append(
      button("Undo AI edit…", () => {
        undo.hidden = !undo.hidden;
      }),
    );
    li.append(actions, undo);
  } else {
    li.append(actions);
  }
  if (h.edited) {
    const c = el("p", "hint");
    c.append("Your correction: ", el("q", undefined, h.edited));
    li.append(c);
  }
  actions.append(
    button(
      "Delete",
      () =>
        void (async () => {
          try {
            await invoke("history_delete", { id: h.id });
            li.remove();
          } catch (e) {
            setStatus("hist_status", String(e), false);
          }
        })(),
      "small danger",
    ),
  );
  return li;
}

let histSeq = 0;
async function loadHistory(): Promise<void> {
  const seq = ++histSeq;
  try {
    const rows = await invoke<HistoryItem[]>("history_list", {
      query: $<HTMLInputElement>("hist_query").value,
      limit: 200,
    });
    if (seq !== histSeq) return; // a newer search is in flight
    const list = $<HTMLOListElement>("hist_list");
    list.replaceChildren(...rows.map(historyRow));
    $<HTMLElement>("hist_empty").hidden = rows.length > 0;
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

let clearArmed = 0;
async function clearHistory(): Promise<void> {
  const b = $<HTMLButtonElement>("hist_clear");
  if (Date.now() - clearArmed > 3000) {
    clearArmed = Date.now();
    b.textContent = "Click again to delete all";
    setTimeout(() => (b.textContent = "Delete all"), 3000);
    return;
  }
  clearArmed = 0;
  b.textContent = "Delete all";
  await act(invoke("history_clear"), "History deleted");
  await loadHistory();
}

async function saveRetention(): Promise<void> {
  if (!current) return;
  try {
    await savePrefs({ ...current, retention: $<HTMLSelectElement>("retention").value });
    setStatus("hist_status", "Saved", true);
    await loadHistory();
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

// ------------------------------------------------------------------ dictionary

let dict: DictEntry[] = [];

function dictRow(e: DictEntry, idx: number): HTMLTableRowElement {
  const tr = el("tr");
  const word = el("input");
  word.value = e.word;
  word.spellcheck = false;
  word.addEventListener("input", () => (dict[idx].word = word.value));
  const rep = el("textarea");
  rep.rows = 1;
  rep.value = e.replacement ?? "";
  rep.placeholder = "(vocabulary word)";
  rep.addEventListener("input", () => (dict[idx].replacement = rep.value || null));
  const snip = el("input");
  snip.type = "checkbox";
  snip.checked = Boolean(e.isSnippet);
  snip.title = "Snippet: the trigger inserts this text";
  snip.addEventListener("change", () => (dict[idx].isSnippet = snip.checked));
  const del = button("✕", () => {
    dict.splice(idx, 1);
    renderDict();
  }, "small danger");
  del.title = "Remove";
  for (const c of [word, rep, snip, del]) {
    const td = el("td");
    td.append(c);
    tr.append(td);
  }
  return tr;
}

function renderDict(): void {
  const f = $<HTMLInputElement>("dict_filter").value.trim().toLowerCase();
  const rows: HTMLTableRowElement[] = [];
  dict.forEach((e, i) => {
    if (!f || e.word.toLowerCase().includes(f) || (e.replacement ?? "").toLowerCase().includes(f))
      rows.push(dictRow(e, i));
  });
  $<HTMLTableSectionElement>("dict_rows").replaceChildren(...rows);
}

async function saveDict(): Promise<void> {
  if (!current) return;
  try {
    await savePrefs({ ...current, dictionary: dict.map((e) => ({ ...e })) });
    // The backend normalises (trims, drops blanks, collapses duplicates).
    current = await invoke<Prefs>("get_prefs");
    dict = current.dictionary.map((e) => ({ ...e }));
    renderDict();
    setStatus("dict_status", "Saved", true);
    await loadSuggestions();
  } catch (e) {
    setStatus("dict_status", String(e), false);
  }
}

async function importFile(): Promise<void> {
  const input = $<HTMLInputElement>("dict_file");
  const file = input.files?.[0];
  if (!file) return;
  try {
    const json = await file.text();
    const r = await invoke<MergeReport>("dictionary_import", {
      entries: dict,
      json,
      replace: $<HTMLSelectElement>("dict_mode").value === "replace",
    });
    dict = r.entries;
    renderDict();
    const skipped = r.skipped ? `, ${r.skipped} invalid skipped` : "";
    setStatus(
      "io_status",
      `${r.added} added, ${r.updated} updated${skipped}. Press "Save dictionary" to keep them.`,
      true,
    );
  } catch (e) {
    setStatus("io_status", String(e), false);
  } finally {
    input.value = "";
  }
}

async function exportFile(): Promise<void> {
  try {
    const path = await invoke<string>("dictionary_export", { entries: dict });
    setStatus("io_status", `Saved ${dict.length} entries to ${path}`, true);
  } catch (e) {
    setStatus("io_status", String(e), false);
  }
}

async function copyJson(): Promise<void> {
  try {
    const text = await invoke<string>("dictionary_export_text", { entries: dict });
    await invoke("clipboard_copy", { text });
    setStatus("io_status", "JSON copied to the clipboard", true);
  } catch (e) {
    setStatus("io_status", String(e), false);
  }
}

// ------------------------------------------------------------------ suggestions

function showSuggestionCount(n: number): void {
  const b = $<HTMLElement>("sugg_badge");
  b.hidden = n === 0;
  b.textContent = String(n);
}

async function loadSuggestions(): Promise<void> {
  const list = await invoke<Suggestion[]>("suggestions_list");
  showSuggestionCount(list.length);
  $<HTMLElement>("sugg_box").hidden = list.length === 0;
  $<HTMLUListElement>("sugg_list").replaceChildren(
    ...list.map((s) => {
      const li = el("li");
      const what = el("span", "what");
      what.append(el("q", undefined, s.heard), " → ", el("strong", undefined, s.word));
      const meta = el("span", "hint", ` ${s.app.replace(/\.exe$/i, "")}${s.count > 1 ? ` ×${s.count}` : ""}`);
      const accept = async (asReplacement: boolean): Promise<void> => {
        try {
          current = await invoke<Prefs>("suggestion_accept", { word: s.word, asReplacement });
          dict = current.dictionary.map((e) => ({ ...e }));
          renderDict();
          setStatus("dict_status", `Added "${s.word}"`, true);
        } catch (e) {
          setStatus("dict_status", String(e), false);
        }
        await loadSuggestions();
      };
      li.append(
        what,
        meta,
        button("Add word", () => void accept(false)),
        button("Add as replacement", () => void accept(true), "small secondary"),
        button(
          "Dismiss",
          () => void invoke("suggestion_dismiss", { word: s.word }).then(loadSuggestions),
          "small secondary",
        ),
      );
      return li;
    }),
  );
}

// ------------------------------------------------------------------ boot

async function load(): Promise<void> {
  const [prefs, mics, model] = await Promise.all([
    invoke<Prefs>("get_prefs"),
    invoke<string[]>("list_microphones"),
    invoke<ModelInfo>("get_model_status"),
  ]);
  current = prefs;
  showModel(model.status);
  showLlm(model.llm);
  $<HTMLElement>("models_dir").textContent = model.models_dir;
  $<HTMLElement>("llm_plugin").textContent = model.llm_plugin;
  $<HTMLInputElement>("model_path").placeholder = model.default_path;
  $<HTMLInputElement>("llm_model_path").placeholder = model.default_llm_path;
  const mic = $<HTMLSelectElement>("microphone");
  mic.replaceChildren(new Option("System default", ""), ...mics.map((m) => new Option(m, m)));
  for (const b of BUCKETS) {
    $<HTMLSelectElement>(`style_${b}`).replaceChildren(
      ...STYLES.map(([v, label]) => new Option(label, v)),
    );
  }
  fillForm(prefs);
  dict = prefs.dictionary.map((e) => ({ ...e }));
  renderDict();
  await loadSuggestions();
}

for (const b of document.querySelectorAll<HTMLButtonElement>("nav.tabs button")) {
  b.addEventListener("click", () => showTab(b.dataset.tab ?? "general"));
}
$<HTMLFormElement>("form").addEventListener("submit", (e) => void saveForm(e));
let searchTimer = 0;
$<HTMLInputElement>("hist_query").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => void loadHistory(), 200);
});
$<HTMLButtonElement>("hist_clear").addEventListener("click", () => void clearHistory());
$<HTMLSelectElement>("retention").addEventListener("change", () => void saveRetention());
$<HTMLInputElement>("dict_filter").addEventListener("input", renderDict);
$<HTMLButtonElement>("dict_add").addEventListener("click", () => {
  $<HTMLInputElement>("dict_filter").value = "";
  dict.unshift({ word: "", replacement: null, isSnippet: false });
  renderDict();
  $<HTMLTableSectionElement>("dict_rows").querySelector("input")?.focus();
});
$<HTMLButtonElement>("dict_save").addEventListener("click", () => void saveDict());
$<HTMLInputElement>("dict_file").addEventListener("change", () => void importFile());
$<HTMLButtonElement>("dict_export").addEventListener("click", () => void exportFile());
$<HTMLButtonElement>("dict_copy").addEventListener("click", () => void copyJson());

void onEvent<ModelStatus>("model:status", (e) => showModel(e.payload));
void onEvent<LlmStatus>("llm:status", (e) => showLlm(e.payload));
void onEvent<number>("suggestions:changed", () => void loadSuggestions());
void onEvent<string>("settings:tab", (e) => showTab(e.payload));
void onEvent<null>("history:changed", () => {
  if (!$<HTMLElement>("hist_list").closest("[hidden]")) void loadHistory();
});

showTab("general");
void load();
