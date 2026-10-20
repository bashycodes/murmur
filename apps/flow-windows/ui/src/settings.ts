// Settings window: edits prefs.json through the `get_prefs` / `set_prefs`
// commands. Field names mirror `src/prefs.rs` (snake_case).

interface DictEntry {
  word: string;
  replacement?: string | null;
  isSnippet?: boolean;
}

interface Prefs {
  shortcuts: { ptt: string; hands_free: string; cancel: string };
  microphone: string | null;
  languages: string[];
  enable_sounds: boolean;
  mute_audio_while_dictating: boolean;
  launch_at_login: boolean;
  show_flow_bar_always: boolean;
  use_ax_context: boolean;
  cleanup_level: string;
  raw_mode: boolean;
  dictionary: DictEntry[];
  paste_method: string;
  restore_clipboard: boolean;
  max_dictation_minutes: number;
  [k: string]: unknown;
}

const { invoke } = window.__TAURI__.core;

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

const BOOL_FIELDS = [
  "enable_sounds",
  "mute_audio_while_dictating",
  "launch_at_login",
  "show_flow_bar_always",
  "use_ax_context",
  "raw_mode",
  "restore_clipboard",
] as const;

let current: Prefs | null = null;

/** One entry per line: `word`, `spoken => replacement`, `trigger ==> snippet`. */
function parseDictionary(text: string): DictEntry[] {
  const out: DictEntry[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const snip = line.split("==>");
    if (snip.length === 2) {
      out.push({ word: snip[0].trim(), replacement: snip[1].trim(), isSnippet: true });
      continue;
    }
    const rep = line.split("=>");
    if (rep.length === 2) {
      out.push({ word: rep[0].trim(), replacement: rep[1].trim(), isSnippet: false });
      continue;
    }
    out.push({ word: line, replacement: null, isSnippet: false });
  }
  return out;
}

function formatDictionary(d: DictEntry[]): string {
  return d
    .map((e) =>
      e.replacement ? `${e.word} ${e.isSnippet ? "==>" : "=>"} ${e.replacement}` : e.word,
    )
    .join("\n");
}

async function load(): Promise<void> {
  const [prefs, mics] = await Promise.all([
    invoke<Prefs>("get_prefs"),
    invoke<string[]>("list_microphones"),
  ]);
  current = prefs;
  $<HTMLInputElement>("ptt").value = prefs.shortcuts.ptt;
  $<HTMLInputElement>("hands_free").value = prefs.shortcuts.hands_free;
  $<HTMLInputElement>("cancel").value = prefs.shortcuts.cancel;
  const mic = $<HTMLSelectElement>("microphone");
  mic.innerHTML = "";
  const def = new Option("System default", "");
  mic.add(def);
  for (const m of mics) mic.add(new Option(m, m));
  mic.value = prefs.microphone ?? "";
  for (const f of BOOL_FIELDS) $<HTMLInputElement>(f).checked = Boolean(prefs[f]);
  $<HTMLSelectElement>("cleanup_level").value = prefs.cleanup_level;
  $<HTMLSelectElement>("paste_method").value = prefs.paste_method;
  $<HTMLInputElement>("languages").value = prefs.languages.join(", ");
  $<HTMLInputElement>("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
  $<HTMLTextAreaElement>("dictionary").value = formatDictionary(prefs.dictionary);
}

async function save(ev: Event): Promise<void> {
  ev.preventDefault();
  if (!current) return;
  const next: Prefs = { ...current };
  next.shortcuts = {
    ptt: $<HTMLInputElement>("ptt").value.trim(),
    hands_free: $<HTMLInputElement>("hands_free").value.trim(),
    cancel: $<HTMLInputElement>("cancel").value.trim(),
  };
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
  next.dictionary = parseDictionary($<HTMLTextAreaElement>("dictionary").value);
  const status = $<HTMLSpanElement>("status");
  try {
    await invoke("set_prefs", { prefs: next });
    current = next;
    status.textContent = "Saved";
    status.className = "ok";
  } catch (e) {
    status.textContent = String(e);
    status.className = "err";
  }
}

$<HTMLFormElement>("form").addEventListener("submit", (e) => void save(e));
void load();
