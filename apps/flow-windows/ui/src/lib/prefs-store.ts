// The single owner of prefs.json on this page: the ONLY caller of
// `set_prefs`. Pages save a patch of the fields they own; the store lays
// it over the latest saved prefs, so one page's save can never roll back
// another page's earlier save (the old single form read every field on
// every Save). Field names mirror `src/prefs.rs` (snake_case).

import { invoke } from "./ipc.js";

export interface DictEntry {
  word: string;
  replacement?: string | null;
  isSnippet?: boolean;
}

export interface Prefs {
  shortcuts: {
    ptt: string;
    hands_free: string;
    command: string;
    cancel: string;
    paste_last: string;
    copy_last: string;
    scratchpad?: string;
    new_note?: string;
  };
  microphone: string | null;
  languages: string[];
  /** D37 "Keyboard language": each dictation uses the foreground window's layout language. */
  keyboard_language?: boolean;
  enable_sounds: boolean;
  mute_audio_while_dictating: boolean;
  launch_at_login: boolean;
  show_flow_bar_always: boolean;
  use_ax_context: boolean;
  cleanup_level: string;
  raw_mode: boolean;
  styles: { personal: string; work: string; email: string; other: string };
  /** "Style per app": exe stem or web host -> bucket (D27). */
  app_buckets: Record<string, string>;
  dictionary: DictEntry[];
  paste_method: string;
  restore_clipboard: boolean;
  max_dictation_minutes: number;
  hands_free_silence_ms: number;
  retention: string;
  /** Recordings kept for History "Retry" (the newest rows; 0 = none). */
  retry_audio_keep?: number;
  model_path: string | null;
  asr_engine: string;
  parakeet_model_dir: string | null;
  /** Model of the Hebrew route (D33): "zipformer" | "parakeet_he" | "none". */
  hebrew_model?: string;
  /** A Hebrew model folder from the server's library (D41); null = the catalogue folder. */
  hebrew_model_dir?: string | null;
  /** Language-ID model from the server's library (D41); null = the default file. */
  lid_model_path?: string | null;
  spoken_cleanup: boolean;
  llm_enabled: boolean;
  llm_model_path: string | null;
  llm_lib_path: string | null;
  llm_timeout_ms: number;
  learn_from_edits: boolean;
  /** First-run setup finished or skipped (false only on a fresh install). */
  onboarded?: boolean;
  transforms?: TransformPrefs;
  [k: string]: unknown;
}

/** One saved transform (`transforms::Item`, src/transforms.rs). */
export interface TransformItem {
  transform: { id: string; name: string; description: string; instruction: string };
  /** "Win+Alt+1"; null = no shortcut. */
  shortcut: string | null;
}

/** `prefs.transforms` (docs/proposals/transforms.md). */
export interface TransformPrefs {
  /** Master "Opt in": off = no shortcut registered, no auto apply. */
  enabled: boolean;
  /** Transform id run on every dictation; null = off. */
  auto_apply: string | null;
  view_changes: string | null;
  items: TransformItem[];
}

type Listener = (p: Prefs) => void;

let current: Prefs | null = null;
const listeners: Listener[] = [];

function publish(p: Prefs): void {
  current = p;
  for (const l of listeners) l(p);
}

export const prefsStore = {
  /** The last saved (or loaded) prefs; null before `load()`. */
  get(): Prefs | null {
    return current;
  },

  /** Called with the new prefs after every load, save or adopt. */
  subscribe(l: Listener): void {
    listeners.push(l);
  },

  async load(): Promise<Prefs> {
    const p = await invoke<Prefs>("get_prefs");
    publish(p);
    return p;
  },

  /** Save `patch` over the latest prefs. Throws the backend's error. */
  async save(patch: Partial<Prefs>): Promise<Prefs> {
    if (!current) throw new Error("prefs not loaded yet");
    const next: Prefs = { ...current, ...patch };
    await invoke("set_prefs", { prefs: next });
    publish(next);
    return next;
  },

  /** Take prefs the backend changed itself (e.g. `suggestion_accept`). */
  adopt(p: Prefs): void {
    publish(p);
  },
};
