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
  hands_free_silence_ms: number;
  retention: string;
  model_path: string | null;
  asr_engine: string;
  parakeet_model_dir: string | null;
  spoken_cleanup: boolean;
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
  | { state: "missing"; path: string; engine?: string }
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
  default_parakeet_dir: string;
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
  "spoken_cleanup",
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
  if (name === "developer") void loadDev();
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

function showEngineFields(): void {
  const engine = $<HTMLSelectElement>("asr_engine").value;
  for (const l of document.querySelectorAll<HTMLElement>("[data-engine]")) {
    l.hidden = l.dataset.engine !== engine;
  }
}

function fillForm(prefs: Prefs): void {
  $<HTMLInputElement>("model_path").value = prefs.model_path ?? "";
  $<HTMLSelectElement>("asr_engine").value = prefs.asr_engine;
  $<HTMLInputElement>("parakeet_model_dir").value = prefs.parakeet_model_dir ?? "";
  showEngineFields();
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
  $<HTMLInputElement>("hands_free_silence_s").value =
    prefs.hands_free_silence_ms > 0 ? String(prefs.hands_free_silence_ms / 1000) : "0";
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
  next.asr_engine = $<HTMLSelectElement>("asr_engine").value;
  next.parakeet_model_dir = $<HTMLInputElement>("parakeet_model_dir").value.trim() || null;
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
  // Hands-free auto-stop (D19): 0 = off, else 0.3-10 s of silence.
  const hf = parseFloat($<HTMLInputElement>("hands_free_silence_s").value);
  next.hands_free_silence_ms =
    !Number.isFinite(hf) || hf <= 0 ? 0 : Math.round(Math.min(10, Math.max(0.3, hf)) * 1000);
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


// ------------------------------------------------------------------ developer
// Hybrid local/server routing (docs/hybrid-architecture.md §5). The backend
// owns validation (`dev_validate`), so the inline errors and Save use the
// same rules; the token is shown here but stored DPAPI-encrypted.

type Route = "server" | "client" | "cloud";

// Router `CloudProviderConfig` (flow-router; no keys, ever).
interface CloudProvider {
  id: string;
  preset: string;
  adapter: string;
  dialect: string;
  label: string;
  base_url: string;
  model: string;
  language: string;
  send_dictionary: boolean;
  send_screen_words: boolean;
  zero_retention: boolean;
  format_override: string;
  timeout_base_ms: number;
  price_per_hour_usd: number | null;
  consent_host: string;
  consent_at_ms: number;
}

interface CloudConfig {
  active: string;
  providers: CloudProvider[];
  monthly_budget_usd: number | null;
}

interface KeyState {
  set: boolean;
  masked: string;
}

interface UsageView {
  month: string;
  rows: { provider_id: string; label: string; requests: number; minutes: number; cost: string }[];
  total: string;
  budget: number | null;
  budget_reached: boolean;
}

interface CloudView {
  capabilities: { cloud: boolean; adapters: string[] };
  keys: Record<string, KeyState>;
  usage: UsageView;
  presets: CloudProvider[];
}

interface DevForm {
  allow_server: boolean;
  allow_client: boolean;
  route_order: Route[];
  host: string;
  port: number;
  tls: boolean;
  tls_pin_sha256: string;
  token: string;
  token_saved?: boolean;
  connect_timeout_ms: number;
  commit_timeout_base_ms: number;
  health_ttl_s: number;
  keep_local_warm: boolean;
  show_engine_badge: boolean;
  dev_unlocked: boolean;
  allow_cloud: boolean;
  cloud: CloudConfig;
}

interface FieldError {
  field: string;
  message: string;
}

interface DevCheck {
  errors: FieldError[];
  warnings: string[];
  tls_forced: boolean;
}

interface Health {
  ok: boolean;
  error: string | null;
  message: string | null;
  server: string | null;
  rtt_ms: { min: number; median: number; max: number } | null;
  server_version: string | null;
  server_core: string | null;
  postprocess_match: boolean | null;
  proto: number | null;
  asr_backend: string | null;
  asr_model: string | null;
  llm: boolean | null;
  workers: number | null;
  busy: number | null;
  provider?: string;
  model?: string;
  transcript?: string;
  latency_ms?: number;
  cost_usd?: number;
}

interface LastDictation {
  engine: string | null;
  fallback: string | null;
  server: string | null;
  rtt_ms: number | null;
  queue_ms: number | null;
  asr_ms: number;
  total_ms: number;
  status: string;
  empty_reason: string | null;
  at_ms: number;
  row: string | null;
  notice: string | null;
}

interface Circuit {
  open: boolean;
  reason: string | null;
  retry_in_ms: number | null;
  until_config_change: boolean;
}

interface RouteView {
  last: LastDictation | null;
  circuit: Circuit;
  health: Health | null;
  cloud_circuit: Circuit | null;
}

interface DevView {
  form: DevForm;
  caps: { local: boolean; remote: boolean; cloud: boolean };
  capabilities: { local: boolean; remote: boolean; asr_engines: string[]; llm: boolean; proto: number; core_version: string };
  visible: boolean;
  version: string;
  check: DevCheck;
  route: RouteView;
  cloud: CloudView;
  keys_cleared?: string[];
}

interface SessionTest {
  status: string;
  empty_reason: string | null;
  error: string | null;
  engine: string | null;
  fallback: string | null;
  total_ms: number;
}

const ROUTE_LABEL: Record<Route, string> = {
  server: "Server",
  client: "This PC (client)",
  cloud: "Cloud provider (third party)",
};
const DEV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"] as const;
const DEV_TEXT = ["host", "tls_pin_sha256", "token"] as const;
const DEV_BOOL = ["allow_server", "allow_client", "tls", "keep_local_warm", "show_engine_badge"] as const;

let devView: DevView | null = null;
let devOrder: Route[] = ["server", "client", "cloud"];
let devValid = true;

const HINTS: Record<string, string> = {
  auth_failed: "token rejected — copy it again from the server's token file",
  connect_failed: "is flow-server running / Tailscale up?",
  proto_mismatch: "client and server protocol versions differ — update one of them",
  server_not_configured: "enter the server host first",
  timeout: "the server is too slow or stuck",
  busy: "the server is busy with another dictation",
};

function hint(code: string | null): string {
  return code ? (HINTS[code] ?? "") : "";
}

function readDev(): DevForm {
  const base = devView?.form;
  const f = { ...(base as DevForm) };
  for (const k of DEV_BOOL) f[k] = $<HTMLInputElement>(`dev_${k}`).checked;
  for (const k of DEV_TEXT) f[k] = $<HTMLInputElement>(`dev_${k}`).value;
  for (const k of DEV_NUM) {
    const v = $<HTMLInputElement>(`dev_${k}`).value.trim();
    f[k] = v === "" ? -1 : Number(v);
  }
  f.route_order = [...devOrder];
  f.allow_cloud = $<HTMLInputElement>("dev_allow_cloud").checked;
  const budget = $<HTMLInputElement>("dev_cloud_budget").value.trim();
  f.cloud = { ...cloudCfg, monthly_budget_usd: budget === "" ? null : Number(budget) };
  return f;
}

function fillDev(f: DevForm): void {
  for (const k of DEV_BOOL) $<HTMLInputElement>(`dev_${k}`).checked = f[k];
  for (const k of DEV_TEXT) $<HTMLInputElement>(`dev_${k}`).value = f[k];
  for (const k of DEV_NUM) $<HTMLInputElement>(`dev_${k}`).value = String(f[k]);
  showTokenSaved(!!f.token_saved);
  devOrder = [...f.route_order];
  $<HTMLInputElement>("dev_allow_cloud").checked = f.allow_cloud;
  cloudCfg = structuredClone(f.cloud);
  $<HTMLInputElement>("dev_cloud_budget").value =
    f.cloud.monthly_budget_usd == null ? "" : String(f.cloud.monthly_budget_usd);
  if (!cloudCfg.providers.some((p) => p.id === cloudSel)) cloudSel = cloudCfg.active || null;
  renderRoutes();
  renderCloud();
}

// The saved token never comes back to the page (S18): an empty field
// keeps it; typing replaces it; Clear removes it on Save.
function showTokenSaved(saved: boolean): void {
  $<HTMLInputElement>("dev_token").placeholder = saved
    ? "saved (hidden) — leave empty to keep it"
    : "from flow-server gen-token";
}

function renderRoutes(): void {
  const srv = $<HTMLInputElement>("dev_allow_server").checked;
  const cli = $<HTMLInputElement>("dev_allow_client").checked;
  const cloud = $<HTMLInputElement>("dev_allow_cloud").checked && !!devView?.caps.cloud;
  const on = (r: Route): boolean => (r === "server" ? srv : r === "client" ? cli : cloud);
  // Cloud is listed only in builds that have it.
  const shown = devOrder.filter((r) => r !== "cloud" || !!devView?.caps.cloud);
  $<HTMLElement>("dev_order_box").hidden = shown.filter(on).length < 2;
  $<HTMLOListElement>("dev_routes").replaceChildren(
    ...shown.map((r, i) => {
      const li = el("li", on(r) ? "" : "off", ROUTE_LABEL[r] + " ");
      if (i > 0) {
        li.append(
          button("Move up", () => {
            const a = devOrder.indexOf(shown[i - 1]);
            const b = devOrder.indexOf(r);
            [devOrder[a], devOrder[b]] = [devOrder[b], devOrder[a]];
            renderRoutes();
            scheduleCheck();
          }, "small secondary"),
        );
      }
      return li;
    }),
  );
  $<HTMLElement>("dev_server_box").hidden = !srv;
  $<HTMLElement>("dev_pin_row").hidden = !$<HTMLInputElement>("dev_tls").checked;
}

function showCheck(c: DevCheck): void {
  for (const s of document.querySelectorAll<HTMLElement>("[data-err]")) s.textContent = "";
  for (const i of document.querySelectorAll<HTMLInputElement>("[data-panel=developer] input")) i.classList.remove("invalid");
  for (const e of c.errors) {
    const slot = document.querySelector<HTMLElement>(`[data-err="${e.field}"]`);
    if (slot) slot.textContent = e.message;
    $<HTMLInputElement>(`dev_${e.field}`)?.classList.add("invalid");
  }
  devValid = c.errors.length === 0;
  $<HTMLButtonElement>("dev_save").disabled = !devValid;
  $<HTMLButtonElement>("dev_test_conn").disabled = !devValid;
  $<HTMLElement>("dev_tls_forced").hidden = !c.tls_forced;
  const tls = $<HTMLInputElement>("dev_tls");
  if (c.tls_forced) tls.checked = true;
  tls.disabled = c.tls_forced;
  $<HTMLUListElement>("dev_warnings").replaceChildren(
    ...c.warnings.map((w) => el("li", w.startsWith("Dictation disabled") ? "err" : "", w)),
  );
}

let checkTimer = 0;
function scheduleCheck(): void {
  renderRoutes();
  clearTimeout(checkTimer);
  checkTimer = setTimeout(() => {
    void invoke<DevCheck>("dev_validate", { form: readDev() }).then(showCheck);
  }, 150);
}

function healthText(h: Health): string {
  if (!h.ok) {
    const code = h.error ?? "error";
    // flow-remote refuses ws:// to an address that is not private/tailnet (S6).
    if (/plaintext (refused|to public address)/.test(h.message ?? "")) {
      return "Server needs TLS — its address is not private (or resolves to a public one); turn on TLS and set a certificate pin";
    }
    const why = hint(code);
    return `${code}${h.message ? `: ${h.message}` : ""}${why ? ` — ${why}` : ""}`;
  }
  const r = h.rtt_ms;
  const parts = [
    r ? `OK · median ${r.median} ms (${r.min}-${r.max})` : "OK",
    h.server_version ?? "",
    h.asr_backend ?? "",
  ].filter(Boolean);
  return parts.join(" · ");
}

function showServerInfo(h: Health | null): void {
  const e = $<HTMLElement>("dev_server_info");
  if (!h) {
    e.textContent = "Not probed yet";
    e.className = "";
    return;
  }
  if (!h.ok) {
    e.textContent = healthText(h);
    e.className = "err";
    return;
  }
  const pp = h.postprocess_match === false ? " · ⚠ server pipeline ≠ client pipeline; results may differ" : " · pipeline ✓";
  e.textContent =
    `${h.server ?? ""} · ${h.server_version ?? "?"} (core ${h.server_core ?? "?"}, proto ${h.proto ?? "?"})` +
    ` · ASR ${h.asr_backend ?? "?"} ${h.asr_model ?? ""} · LLM ${h.llm ? "on" : "off"}` +
    ` · workers ${h.workers ?? "?"} (busy ${h.busy ?? "?"})${pp}`;
  e.className = h.postprocess_match === false ? "err" : "ok";
}

function showRoute(r: RouteView): void {
  const last = r.last;
  const e = $<HTMLElement>("dev_last");
  if (!last) {
    e.textContent = "No dictation yet";
  } else {
    const engine =
      last.engine === "remote" ? "server" : last.engine === "local" ? "client" : (last.engine ?? "none");
    const bits = [
      last.row ?? engine,
      last.notice ?? "",
      last.fallback ? `fallback: ${last.fallback}` : "",
      last.server ? `server ${last.server}` : "",
      last.rtt_ms != null ? `rtt ${last.rtt_ms} ms` : "",
      last.queue_ms != null ? `queue ${last.queue_ms} ms` : "",
      `asr ${last.asr_ms} ms / total ${last.total_ms} ms`,
      last.status,
      last.empty_reason ?? "",
      new Date(last.at_ms).toLocaleTimeString(),
    ].filter(Boolean);
    e.textContent = bits.join(" · ");
  }
  const c = r.circuit;
  $<HTMLElement>("dev_circuit").textContent = !c.open
    ? "closed"
    : c.until_config_change
      ? `open until the server settings change (${c.reason ?? "?"})`
      : `open until ${new Date(Date.now() + (c.retry_in_ms ?? 0)).toLocaleTimeString()} (${c.reason ?? "?"})`;
  showServerInfo(r.health);
  const cc = r.cloud_circuit;
  $<HTMLElement>("dev_cloud_circuit_dt").hidden = !cc;
  $<HTMLElement>("dev_cloud_circuit").hidden = !cc;
  if (cc) {
    $<HTMLElement>("dev_cloud_circuit").textContent = !cc.open
      ? "closed"
      : cc.until_config_change
        ? `open until the key or provider changes (${cloudHint(cc.reason)})`
        : `open until ${new Date(Date.now() + (cc.retry_in_ms ?? 0)).toLocaleTimeString()} (${cloudHint(cc.reason)})`;
  }
}

function showDev(v: DevView): void {
  devView = v;
  $<HTMLElement>("tab_developer").hidden = !v.visible;
  $<HTMLElement>("version_row").textContent = `Flow Clone ${v.version}`;
  $<HTMLElement>("privacy_line").textContent = v.caps.remote
    ? "Speech is processed on this PC or on your own server. Nothing goes to a third party."
    : "Everything runs on this PC. Nothing is sent over the network.";
  fillDev(v.form);
  // Server-only builds have no local model: no model paths or LLM files.
  for (const f of document.querySelectorAll<HTMLElement>("[data-needs=local]")) f.hidden = !v.caps.local;
  $<HTMLInputElement>("dev_allow_server").disabled = !v.caps.remote;
  $<HTMLInputElement>("dev_allow_client").disabled = !v.caps.local;
  $<HTMLElement>("dev_server_na").hidden = v.caps.remote;
  $<HTMLElement>("dev_client_na").hidden = v.caps.local;
  $<HTMLElement>("dev_lock").hidden = v.caps.remote;
  $<HTMLElement>("dev_cloud_box").hidden = !v.caps.cloud;
  if (v.caps.cloud) {
    $<HTMLElement>("privacy_line").textContent =
      "Speech is processed on this PC or on your own server, and by a cloud provider only if you turn it on in Developer settings.";
  }
  cloudKeys = { ...v.cloud.keys };
  cloudPresets = v.cloud.presets;
  $<HTMLSelectElement>("dev_cloud_preset").replaceChildren(
    ...v.cloud.presets.map((p) => new Option(p.preset === "custom" ? "Custom (OpenAI-compatible URL)" : p.label, p.preset)),
  );
  showUsage(v.cloud.usage);
  const c = v.capabilities;
  $<HTMLElement>("dev_build").textContent =
    `${[c.local ? "client" : "", c.remote ? "server" : ""].filter(Boolean).join(" + ") || "no engine"}` +
    ` · ASR ${c.asr_engines.join(", ") || "none"} · LLM ${c.llm ? "yes" : "no"} · proto ${c.proto} · core ${c.core_version}`;
  showCheck(v.check);
  showRoute(v.route);
}

async function loadDev(): Promise<void> {
  try {
    showDev(await invoke<DevView>("dev_get"));
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
}

async function devCall<T>(cmd: string, args: Record<string, unknown>, okText: string): Promise<T | null> {
  try {
    const r = await invoke<T>(cmd, args);
    setStatus("dev_status", okText, true);
    return r;
  } catch (e) {
    const errs = e as FieldError[] | string;
    if (Array.isArray(errs)) {
      showCheck({ errors: errs, warnings: [], tls_forced: false });
      setStatus("dev_status", "Fix the highlighted fields", false);
    } else {
      setStatus("dev_status", String(e), false);
    }
    return null;
  }
}

async function devSave(): Promise<void> {
  if (!devValid) return;
  // Allowing the cloud with an active provider that has no consent for its
  // current host asks first (a changed host or adapter cleared it).
  const active = activeProvider();
  if ($<HTMLInputElement>("dev_allow_cloud").checked && active && !consentOk(active)) {
    if (!(await askConsent(active))) return;
  }
  const v = await devCall<DevView>("dev_set", { form: readDev() }, "Saved — applies to the next dictation");
  if (v) {
    showDev(v);
    const cleared = v.keys_cleared ?? [];
    if (cleared.length > 0) {
      // A key is bound to the host + adapter it was saved for (S4).
      const names = cleared
        .map((id) => v.form.cloud.providers.find((p) => p.id === id)?.label || id)
        .join(", ");
      setStatus(
        "dev_status",
        `Saved. The API key of ${names} was removed because its host or adapter changed — enter the key again.`,
        false,
      );
    }
  }
}

async function devTestConnection(): Promise<void> {
  if (!devValid) return;
  const out = $<HTMLElement>("dev_test");
  out.textContent = "Testing…";
  out.className = "hint";
  try {
    const h = await invoke<Health>("dev_test_connection", { form: readDev() });
    out.textContent = `Test connection: ${healthText(h)}`;
    out.className = h.ok ? "hint ok" : "hint err";
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
}

async function devTestSession(): Promise<void> {
  const out = $<HTMLElement>("dev_test");
  out.textContent = "Running a 1 s silent session through the saved settings…";
  out.className = "hint";
  try {
    const r = await invoke<SessionTest>("dev_test_session");
    const engine = r.engine === "remote" ? "server" : r.engine === "local" ? "client" : "none";
    const ok = r.status === "EMPTY" && r.empty_reason === "no_speech";
    out.textContent =
      `Test session: ${r.status}${r.empty_reason ? ` (${r.empty_reason})` : ""} via ${engine}` +
      `${r.fallback ? ` (fallback: ${r.fallback} — ${hint(r.fallback)})` : ""} in ${r.total_ms} ms` +
      `${r.error ? ` — ${r.error}` : ""}`;
    out.className = ok ? "hint ok" : "hint err";
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
  void refreshRoute();
}

async function refreshRoute(): Promise<void> {
  try {
    showRoute(await invoke<RouteView>("dev_route_status"));
  } catch {
    /* tab not loaded yet */
  }
}

// ------------------------------------------------- cloud providers (D18)
// Provider edits live in `cloudCfg` until Save (`dev_set`). Keys never
// come back from the backend: only set / ••••last4 (KeyState).

let cloudCfg: CloudConfig = { active: "", providers: [], monthly_budget_usd: null };
let cloudSel: string | null = null;
let cloudKeys: Record<string, KeyState> = {};
let cloudPresets: CloudProvider[] = [];

const CLOUD_HINTS: Record<string, string> = {
  auth_failed: "key rejected",
  quota_exceeded: "out of credits",
  connect_failed: "no internet / base URL wrong",
  budget_exceeded: "monthly budget reached",
  cloud_not_configured: "set a key and accept the consent",
  provider_error: "the provider returned an error",
  timeout: "the provider took too long",
  busy: "rate limited by the provider",
};

// What each provider keeps (docs/cloud-stt.md §2), for the consent dialog.
const RETENTION: Record<string, string> = {
  openrouter: "OpenRouter keeps no audio unless your account opts into logging; with zero retention on, only zero-data-retention upstreams are used.",
  elevenlabs: "ElevenLabs keeps logs/history by default; zero retention needs an enterprise plan.",
  openai_compat: "Retention follows the provider's API terms (Groq: up to 30 days unless ZDR is on in Data Controls; OpenAI: no training by default).",
  deepgram: "Deepgram may use audio for model improvement unless opted out (zero retention on sends mip_opt_out).",
  assemblyai: "AssemblyAI deletes audio in 24-48 h and transcripts after 30 days; zero retention deletes the transcript right after use.",
};

const ZDR_TEXT: Record<string, string> = {
  openrouter: "Zero data retention: route only to providers that keep nothing (provider.zdr)",
  elevenlabs: "Zero retention (enable_logging=false; needs an ElevenLabs enterprise plan)",
  deepgram: "Opt out of Deepgram's model-improvement programme (mip_opt_out)",
  assemblyai: "Delete the transcript at AssemblyAI right after it is fetched",
};

function cloudHint(code: string | null): string {
  return code ? (CLOUD_HINTS[code] ?? code) : "?";
}

function hostOf(url: string): string {
  const m = /^https?:\/\/(\[[^\]]*\]|[^/:?#]*)/i.exec(url.trim());
  return m ? m[1].replace(/^\[|\]$/g, "").toLowerCase() : "";
}

function consentOk(p: CloudProvider): boolean {
  const h = hostOf(p.base_url);
  return p.consent_at_ms > 0 && h !== "" && p.consent_host.toLowerCase() === h;
}

function activeProvider(): CloudProvider | null {
  return cloudCfg.providers.find((p) => p.id === cloudCfg.active) ?? null;
}

function selectedProvider(): CloudProvider | null {
  return cloudCfg.providers.find((p) => p.id === cloudSel) ?? null;
}

function providerName(p: CloudProvider): string {
  return `${p.label || p.id} (${hostOf(p.base_url) || "no host"})`;
}

/** "Send audio to <provider>?" Records consent for the current host. */
function askConsent(p: CloudProvider): Promise<boolean> {
  const d = $<HTMLDialogElement>("dev_consent");
  const name = p.label || p.id;
  $<HTMLElement>("dev_consent_title").textContent = `Send audio to ${name}?`;
  $<HTMLElement>("dev_consent_body").textContent = cloudBanner(p);
  $<HTMLElement>("dev_consent_retention").textContent = RETENTION[p.adapter] ?? "";
  $<HTMLButtonElement>("dev_consent_ok").textContent = `Send audio to ${name}`;
  return new Promise((resolve) => {
    const done = (ok: boolean): void => {
      $<HTMLButtonElement>("dev_consent_ok").onclick = null;
      $<HTMLButtonElement>("dev_consent_cancel").onclick = null;
      d.close();
      if (ok) {
        p.consent_host = hostOf(p.base_url);
        p.consent_at_ms = Date.now();
        renderCloud();
        scheduleCheck();
      }
      resolve(ok);
    };
    $<HTMLButtonElement>("dev_consent_ok").onclick = () => done(true);
    $<HTMLButtonElement>("dev_consent_cancel").onclick = () => done(false);
    d.oncancel = () => done(false);
    d.showModal();
  });
}

function cloudBanner(p: CloudProvider | null): string {
  const who = p ? providerName(p) : "the provider you choose";
  return (
    `Off by default. When on, the audio of your dictations leaves this device and is sent to ${who}, ` +
    "along with your dictionary words. It is processed under that provider's terms, not by your server."
  );
}

function renderCloud(): void {
  $<HTMLElement>("dev_cloud_banner").textContent = cloudBanner(activeProvider());
  $<HTMLUListElement>("dev_cloud_list").replaceChildren(
    ...cloudCfg.providers.map((p) => {
      const li = el("li", p.id === cloudSel ? "selected" : "");
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "dev_cloud_active";
      radio.title = "Use this provider for the cloud route";
      radio.checked = p.id === cloudCfg.active;
      radio.addEventListener("change", () => {
        cloudCfg.active = p.id;
        renderCloud();
        scheduleCheck();
      });
      const k = cloudKeys[p.id];
      const status = [
        p.model,
        k?.set ? `key ${k.masked}` : "no key",
        consentOk(p) ? "consent ✓" : "no consent",
      ].join(" · ");
      li.append(
        radio,
        el("span", "grow", `${providerName(p)} — ${status}`),
        button("Edit", () => {
          cloudSel = p.id;
          renderCloud();
        }, "small secondary"),
        button("Delete", () => void deleteProvider(p.id), "small secondary danger"),
      );
      return li;
    }),
  );
  const p = selectedProvider();
  $<HTMLElement>("dev_cloud_edit").hidden = !p;
  if (!p) return;
  $<HTMLElement>("dev_cloud_edit_title").textContent = `${p.label || p.id} (${p.preset}, ${p.adapter})`;
  for (const f of ["label", "language", "base_url", "model"] as const) {
    const i = $<HTMLInputElement>(`dev_cloud_${f}`);
    if (document.activeElement !== i) i.value = p[f];
  }
  // Fixed endpoints for the dedicated adapters; editable for OpenAI-compatible ones.
  $<HTMLInputElement>("dev_cloud_base_url").readOnly = p.adapter !== "openai_compat";
  for (const f of ["send_dictionary", "send_screen_words", "zero_retention"] as const) {
    $<HTMLInputElement>(`dev_cloud_${f}`).checked = p[f];
  }
  const zdr = ZDR_TEXT[p.adapter];
  $<HTMLElement>("dev_cloud_zdr_row").hidden = !zdr;
  $<HTMLElement>("dev_cloud_zdr_text").textContent = zdr ?? "";
  $<HTMLElement>("dev_cloud_consent_line").textContent = consentOk(p)
    ? `You agreed to send audio to ${p.consent_host} on ${new Date(p.consent_at_ms).toLocaleDateString()}.`
    : `Not yet agreed: the first use asks before any audio goes to ${hostOf(p.base_url) || "this provider"}.`;
  const k = cloudKeys[p.id];
  $<HTMLElement>("dev_cloud_key_state").textContent = k?.set ? `Key set: ${k.masked}` : "No key saved";
}

/** Text field edits on the selected provider. A new host or adapter voids consent. */
function cloudFieldChanged(f: "label" | "language" | "base_url" | "model"): void {
  const p = selectedProvider();
  if (!p) return;
  const v = $<HTMLInputElement>(`dev_cloud_${f}`).value;
  if (f === "base_url" && hostOf(v) !== hostOf(p.base_url)) {
    p.consent_host = "";
    p.consent_at_ms = 0;
  }
  p[f] = v;
  renderCloud();
}

async function addProvider(): Promise<void> {
  const preset = $<HTMLSelectElement>("dev_cloud_preset").value;
  const base = cloudPresets.find((p) => p.preset === preset);
  if (!base) return;
  const id = await invoke<string>("dev_cloud_new_id", {
    preset,
    taken: cloudCfg.providers.map((p) => p.id),
  });
  const p: CloudProvider = { ...structuredClone(base), id, consent_host: "", consent_at_ms: 0 };
  cloudCfg.providers.push(p);
  if (!cloudCfg.active) cloudCfg.active = id;
  cloudSel = id;
  renderCloud();
  scheduleCheck();
}

async function deleteProvider(id: string): Promise<void> {
  cloudCfg.providers = cloudCfg.providers.filter((p) => p.id !== id);
  if (cloudCfg.active === id) cloudCfg.active = cloudCfg.providers[0]?.id ?? "";
  if (cloudSel === id) cloudSel = null;
  // The key leaves Credential Manager now; Save drops the provider.
  try {
    cloudKeys[id] = await invoke<KeyState>("dev_cloud_clear_key", { providerId: id });
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
  renderCloud();
  scheduleCheck();
}

async function saveKey(): Promise<void> {
  const p = selectedProvider();
  const input = $<HTMLInputElement>("dev_cloud_key");
  if (!p || input.value.trim() === "") return;
  try {
    // Bound to this form's host + adapter: the key is only sent there.
    cloudKeys[p.id] = await invoke<KeyState>("dev_cloud_set_key", { providerId: p.id, key: input.value, provider: p });
    input.value = "";
    setStatus("dev_status", "Key saved in Credential Manager", true);
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
  renderCloud();
  scheduleCheck();
}

async function clearKey(): Promise<void> {
  const p = selectedProvider();
  if (!p) return;
  try {
    cloudKeys[p.id] = await invoke<KeyState>("dev_cloud_clear_key", { providerId: p.id });
    setStatus("dev_status", "Key removed", true);
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
  renderCloud();
  scheduleCheck();
}

async function testProvider(): Promise<void> {
  const p = selectedProvider();
  if (!p) return;
  const out = $<HTMLElement>("dev_cloud_test_out");
  out.textContent = `Sending a 3 s public-domain clip to ${hostOf(p.base_url)}…`;
  out.className = "hint";
  try {
    const key = $<HTMLInputElement>("dev_cloud_key").value;
    const h = await invoke<Health>("dev_cloud_test", { provider: p, key: key.trim() === "" ? null : key });
    if (h.ok) {
      const cost = h.cost_usd != null ? ` · $${h.cost_usd.toFixed(5)}` : "";
      out.textContent = `OK · "${h.transcript ?? ""}" · ${h.latency_ms ?? "?"} ms${cost}`;
      out.className = "hint ok";
    } else {
      const code = h.error ?? "error";
      out.textContent = `${code} — ${cloudHint(code)}${h.message ? ` (${h.message})` : ""}`;
      out.className = "hint err";
    }
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
}

function showUsage(u: UsageView): void {
  $<HTMLElement>("dev_cloud_month").textContent = u.month;
  $<HTMLElement>("dev_cloud_total").textContent =
    u.total + (u.budget != null ? ` of $${u.budget.toFixed(2)}${u.budget_reached ? " — budget reached, cloud paused" : ""}` : "");
  $<HTMLUListElement>("dev_cloud_usage").replaceChildren(
    ...u.rows.map((r) => el("li", "", `${r.label}: ${r.requests} dictations · ${r.minutes} min · ${r.cost}`)),
  );
}

let versionTaps = 0;
let versionTimer = 0;
function versionTapped(): void {
  if (!devView || devView.visible) return;
  versionTaps++;
  clearTimeout(versionTimer);
  versionTimer = setTimeout(() => (versionTaps = 0), 3000);
  if (versionTaps >= 7) {
    versionTaps = 0;
    void devCall<DevView>("dev_unlock", { unlocked: true }, "Developer settings unlocked").then((v) => {
      if (v) {
        showDev(v);
        showTab("developer");
      }
    });
  }
}

// ------------------------------------------------------------- debug log

interface DebugLogView {
  enabled: boolean;
  count: number | null;
  path: string;
  stats: { written: number; dropped: number; write_errors: number };
}

function showDebugLog(v: DebugLogView): void {
  $<HTMLInputElement>("dev_debug_log").checked = v.enabled;
  $<HTMLElement>("dev_debug_path").textContent = v.path;
  const n = v.count;
  let line = n === null ? "Log file not readable" : n === 0 ? "No entries" : n === 1 ? "1 entry" : `${n} entries`;
  if (v.stats.write_errors > 0) line += ` · ${v.stats.write_errors} write errors (ignored)`;
  if (v.stats.dropped > 0) line += ` · ${v.stats.dropped} dropped`;
  $<HTMLElement>("dev_debug_count").textContent = line;
}

async function loadDebugLog(): Promise<void> {
  try {
    showDebugLog(await invoke<DebugLogView>("debug_log_status"));
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
}

/** Turning the log off offers to delete what it already holds. */
function askClearDebugLog(count: number): Promise<boolean> {
  const d = $<HTMLDialogElement>("dev_debug_off");
  $<HTMLElement>("dev_debug_off_body").textContent =
    `The log still holds ${count === 1 ? "1 entry" : `${count} entries`} with transcripts of earlier dictations. Delete it now?`;
  return new Promise((resolve) => {
    const done = (ok: boolean): void => {
      $<HTMLButtonElement>("dev_debug_delete").onclick = null;
      $<HTMLButtonElement>("dev_debug_keep").onclick = null;
      d.close();
      resolve(ok);
    };
    $<HTMLButtonElement>("dev_debug_delete").onclick = () => done(true);
    $<HTMLButtonElement>("dev_debug_keep").onclick = () => done(false);
    d.oncancel = () => done(false);
    d.showModal();
  });
}

async function debugLogToggled(): Promise<void> {
  const on = $<HTMLInputElement>("dev_debug_log").checked;
  const v = await devCall<DebugLogView>(
    "debug_log_set",
    { enabled: on },
    on ? "Debug log on: transcripts are being written" : "Debug log off",
  );
  if (!v) {
    $<HTMLInputElement>("dev_debug_log").checked = !on;
    return;
  }
  showDebugLog(v);
  if (!on && (v.count ?? 0) > 0 && (await askClearDebugLog(v.count ?? 0))) {
    const c = await devCall<DebugLogView>("debug_log_clear", {}, "Debug log off and deleted");
    if (c) showDebugLog(c);
  }
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
  $<HTMLInputElement>("parakeet_model_dir").placeholder = model.default_parakeet_dir;
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
  await loadDev();
  await loadDebugLog();
}

for (const b of document.querySelectorAll<HTMLButtonElement>("nav.tabs button")) {
  b.addEventListener("click", () => showTab(b.dataset.tab ?? "general"));
}
$<HTMLFormElement>("form").addEventListener("submit", (e) => void saveForm(e));
$<HTMLSelectElement>("asr_engine").addEventListener("change", showEngineFields);
for (const i of document.querySelectorAll<HTMLInputElement>("[data-panel=developer] input")) {
  i.addEventListener("input", scheduleCheck);
  i.addEventListener("change", scheduleCheck);
}
$<HTMLButtonElement>("dev_save").addEventListener("click", () => void devSave());
$<HTMLInputElement>("dev_debug_log").addEventListener("change", () => void debugLogToggled());
$<HTMLButtonElement>("dev_debug_open").addEventListener("click", () =>
  void devCall<null>("debug_log_open_folder", {}, "Opened the log folder"),
);
$<HTMLButtonElement>("dev_debug_clear").addEventListener("click", () =>
  void devCall<DebugLogView>("debug_log_clear", {}, "Debug log deleted").then((v) => v && showDebugLog(v)),
);
$<HTMLButtonElement>("dev_test_conn").addEventListener("click", () => void devTestConnection());
$<HTMLButtonElement>("dev_test_session").addEventListener("click", () => void devTestSession());
$<HTMLButtonElement>("dev_reset_circuit").addEventListener("click", () =>
  void devCall<RouteView>("dev_reset_circuit", {}, "Circuit reset").then((r) => r && showRoute(r)),
);
$<HTMLButtonElement>("dev_reset").addEventListener("click", () =>
  void devCall<DevView>("dev_reset_defaults", {}, "Defaults restored").then((v) => v && showDev(v)),
);
$<HTMLButtonElement>("dev_lock").addEventListener("click", () =>
  void devCall<DevView>("dev_unlock", { unlocked: false }, "Developer settings locked").then((v) => {
    if (v) {
      showDev(v);
      showTab("general");
    }
  }),
);
$<HTMLButtonElement>("dev_token_show").addEventListener("click", () => {
  const t = $<HTMLInputElement>("dev_token");
  t.type = t.type === "password" ? "text" : "password";
  $<HTMLButtonElement>("dev_token_show").textContent = t.type === "password" ? "Show" : "Hide";
});
$<HTMLButtonElement>("dev_token_paste").addEventListener("click", () => {
  void navigator.clipboard
    .readText()
    .then((t) => {
      $<HTMLInputElement>("dev_token").value = t.trim();
      scheduleCheck();
    })
    .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
});
$<HTMLButtonElement>("dev_token_clear").addEventListener("click", () => {
  $<HTMLInputElement>("dev_token").value = "";
  if (devView) devView.form.token_saved = false;
  showTokenSaved(false);
  scheduleCheck();
});
$<HTMLElement>("version_row").addEventListener("click", versionTapped);
$<HTMLInputElement>("dev_allow_cloud").addEventListener("change", () => {
  const box = $<HTMLInputElement>("dev_allow_cloud");
  const p = activeProvider();
  // Turning cloud on puts it first in the route order (§3.1); the user can reorder.
  if (box.checked) {
    devOrder = ["cloud", ...devOrder.filter((r) => r !== "cloud")];
    if (p && !consentOk(p)) {
      void askConsent(p).then((ok) => {
        if (!ok) box.checked = false;
        scheduleCheck();
      });
    }
  }
  renderCloud();
});
for (const f of ["label", "language", "base_url", "model"] as const) {
  $<HTMLInputElement>(`dev_cloud_${f}`).addEventListener("input", () => cloudFieldChanged(f));
}
for (const f of ["send_dictionary", "send_screen_words", "zero_retention"] as const) {
  $<HTMLInputElement>(`dev_cloud_${f}`).addEventListener("change", () => {
    const p = selectedProvider();
    if (p) p[f] = $<HTMLInputElement>(`dev_cloud_${f}`).checked;
  });
}
$<HTMLButtonElement>("dev_cloud_add").addEventListener("click", () => void addProvider());
$<HTMLButtonElement>("dev_cloud_key_save").addEventListener("click", () => void saveKey());
$<HTMLButtonElement>("dev_cloud_key_clear").addEventListener("click", () => void clearKey());
$<HTMLButtonElement>("dev_cloud_key_paste").addEventListener("click", () => {
  void navigator.clipboard
    .readText()
    .then((t) => ($<HTMLInputElement>("dev_cloud_key").value = t.trim()))
    .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
});
$<HTMLButtonElement>("dev_cloud_test").addEventListener("click", () => void testProvider());
$<HTMLButtonElement>("dev_cloud_reset_usage").addEventListener("click", () =>
  void devCall<UsageView>("dev_cloud_reset_usage", {}, "Usage counters reset").then((u) => u && showUsage(u)),
);
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
void onEvent<RouteView>("dev:route", (e) => {
  showRoute(e.payload);
  if (devView?.caps.cloud) void invoke<UsageView>("dev_cloud_usage").then(showUsage);
});
void onEvent<null>("history:changed", () => {
  if ($<HTMLInputElement>("dev_debug_log").checked) void loadDebugLog();
  if (!$<HTMLElement>("hist_list").closest("[hidden]")) void loadHistory();
});

showTab("general");
void load();
