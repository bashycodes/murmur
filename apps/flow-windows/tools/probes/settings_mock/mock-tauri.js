// Browser stand-in for window.__TAURI__ so ui/dist/settings.html can be
// opened without Tauri (layout / runtime-error check). Probe only: the
// data below is invented and nothing here ships.
(() => {
  const now = Date.now();
  let prefs = {
    shortcuts: { ptt: "Ctrl+Win", hands_free: "Ctrl+Win+Space", cancel: "Esc", paste_last: "Alt+Shift+Z", copy_last: "Alt+Shift+X" },
    microphone: null, languages: [], enable_sounds: true, mute_audio_while_dictating: true,
    launch_at_login: false, show_flow_bar_always: true, use_ax_context: true,
    cleanup_level: "light", raw_mode: false,
    styles: { personal: "casual", work: "formal", email: "formal", other: "formal" },
    dictionary: [
      { word: "Kubernetes", replacement: null, isSnippet: false },
      { word: "gh", replacement: "GitHub", isSnippet: false },
      { word: "my address", replacement: "1 Main Street\nSpringfield", isSnippet: true },
    ],
    paste_method: "ctrl_v", restore_clipboard: true, max_dictation_minutes: 20, retention: "keep",
    model_path: null, live_preview: true, llm_enabled: true, llm_model_path: null, llm_lib_path: null,
    llm_timeout_ms: 1500, learn_from_edits: true,
  };
  let history = [
    { id: 3, ts: now - 60e3, app: "outlook.exe", url: null, raw: "um so i think we should uh meet tomorrow at three",
      formatted: "I think we should meet tomorrow at 3.", pasted: "I think we should meet tomorrow at 3.", edited: null,
      duration_ms: 4100, words: 8, lang: "en", final_text: "I think we should meet tomorrow at 3.", ai_edited: true },
    { id: 2, ts: now - 3600e3, app: "slack.exe", url: null, raw: "i use whisper flow daily",
      formatted: null, pasted: "I use whisper flow daily", edited: "I use Wispr Flow daily",
      duration_ms: 2000, words: 5, lang: "en", final_text: "I use whisper flow daily", ai_edited: false },
    { id: 1, ts: now - 86400e3 * 2, app: "notepad.exe", url: null, raw: "hello world", formatted: null, pasted: null,
      edited: null, duration_ms: 900, words: 2, lang: "en", final_text: "hello world", ai_edited: false },
  ];
  let suggestions = [{ word: "Wispr Flow", heard: "whisper flow", app: "slack.exe", ts: now, count: 2 }];
  const listeners = {};
  const cmds = {
    get_prefs: () => structuredClone(prefs),
    set_prefs: ({ prefs: p }) => { prefs = structuredClone(p); },
    list_microphones: () => ["USB Mic", "Realtek"],
    get_model_status: () => ({
      status: { state: "ready", path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\ggml-base.bin", load_ms: 312 },
      default_path: "C:\\…\\ggml-base.bin", models_dir: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models",
      llm: { state: "plugin_missing", path: "C:\\…\\qwen.gguf", message: "llm plugin not found (C:\\Program Files\\Flow Clone\\flow_llm.dll: not found)" },
      default_llm_path: "C:\\…\\qwen2.5-0.5b-instruct-q4_k_m.gguf", llm_plugin: "flow_llm.dll",
    }),
    history_list: ({ query }) => history.filter((h) => !query || JSON.stringify(h).toLowerCase().includes(query.toLowerCase())),
    history_delete: ({ id }) => { history = history.filter((h) => h.id !== id); return true; },
    history_clear: () => { const n = history.length; history = []; return n; },
    history_copy: () => {}, history_repaste: () => {}, clipboard_copy: () => {},
    suggestions_list: () => suggestions,
    suggestion_accept: ({ word }) => {
      suggestions = suggestions.filter((s) => s.word !== word);
      prefs.dictionary.push({ word, replacement: null, isSnippet: false });
      return structuredClone(prefs);
    },
    suggestion_dismiss: ({ word }) => { suggestions = suggestions.filter((s) => s.word !== word); },
    dictionary_import: ({ entries, json }) => ({ entries: entries.concat(JSON.parse(json)), added: 1, updated: 0, skipped: 0 }),
    dictionary_export: () => "C:\\Users\\u\\Downloads\\flow-dictionary.json",
    dictionary_export_text: ({ entries }) => JSON.stringify(entries, null, 2),
  };
  window.__mockCalls = [];
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__mockCalls.push([cmd, args]);
        if (!cmds[cmd]) throw new Error(`mock: unknown command ${cmd}`);
        return cmds[cmd](args || {});
      },
    },
    event: {
      listen: async (name, cb) => { (listeners[name] ||= []).push(cb); return () => {}; },
    },
  };
  window.__emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
})();
// ?tab=history etc.: open that tab once the page has booted (screenshots).
window.addEventListener("load", () => {
  const tab = new URLSearchParams(location.search).get("tab");
  if (tab) setTimeout(() => document.querySelector(`nav.tabs [data-tab="${tab}"]`)?.click(), 50);
  const undo = new URLSearchParams(location.search).get("undo");
  if (undo) setTimeout(() => [...document.querySelectorAll("button")].find((b) => b.textContent.startsWith("Undo"))?.click(), 300);
});
window.addEventListener("error", (e) => console.error("PAGE ERROR", e.message));
window.addEventListener("unhandledrejection", (e) => console.error("UNHANDLED", String(e.reason)));
