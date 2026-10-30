// Browser stand-in for window.__TAURI__ so ui/dist/settings.html can be
// opened without Tauri (layout / runtime-error check). Probe only: the
// data below is invented and nothing here ships.
(() => {
  const now = Date.now();
  let prefs = {
    shortcuts: { ptt: "Ctrl+Win", hands_free: "Ctrl+Win+Space", command: "Ctrl+Win+Alt", cancel: "Esc", paste_last: "Alt+Shift+Z", copy_last: "Alt+Shift+X" },
    microphone: null, languages: [], enable_sounds: true, mute_audio_while_dictating: true,
    launch_at_login: false, show_flow_bar_always: true, use_ax_context: true,
    cleanup_level: "light", raw_mode: false,
    styles: { personal: "casual", work: "formal", email: "formal", other: "formal" },
    app_buckets: { "mychat.exe": "work", "mail.example.com": "email", slack: "email" },
    dictionary: [
      { word: "Kubernetes", replacement: null, isSnippet: false },
      { word: "gh", replacement: "GitHub", isSnippet: false },
      { word: "my address", replacement: "1 Main Street\nSpringfield", isSnippet: true },
    ],
    paste_method: "ctrl_v", restore_clipboard: true, max_dictation_minutes: 20, hands_free_silence_ms: 2000, retention: "keep",
    model_path: null, live_preview: true, llm_enabled: true, llm_model_path: null, llm_lib_path: null,
    llm_timeout_ms: 1500, learn_from_edits: true,
    asr_engine: "whisper", parakeet_model_dir: null, spoken_cleanup: true,
  };
  let dev = {
    allow_server: true, allow_client: true, route_order: ["server", "client", "cloud"], host: "100.64.0.10", port: 7313,
    tls: false, tls_pin_sha256: "", token: "dev-token-0123456789abcdef", connect_timeout_ms: 800,
    commit_timeout_base_ms: 3000, health_ttl_s: 30, keep_local_warm: true, show_engine_badge: true, dev_unlocked: false,
    decode_while_speaking: true, parakeet_decoding: "auto",
  };
  // D28 OpenRouter: one key (never returned), two consented uses, a catalogue.
  let orKey = "";
  const keyState = () => (orKey ? { set: true, masked: "••••" + orKey.slice(-4) } : { set: false, masked: "" });
  const row = (id, name, o) => ({
    id, name, context_length: 0, input: ["text"], output: ["text"], stt_path: null, price: {}, zdr: true,
    zdr_providers: ["groq"], fast_zdr_providers: [], reasoning_mandatory: false, free: false, allowed_by_account: null,
    recommended: false, price_label: "", badges: ["ZDR"], ...o,
  });
  const catalog = [
    row("openai/whisper-large-v3-turbo", "Whisper Large v3 Turbo", { input: ["audio"], stt_path: "transcriptions", recommended: true, price_label: "$0.04/h audio", badges: ["ZDR", "fast"] }),
    row("elevenlabs/scribe-v2", "ElevenLabs Scribe v2", { input: ["audio"], stt_path: "transcriptions", recommended: true, price_label: "$0.22/h audio" }),
    row("nvidia/parakeet-tdt-0.6b-v3", "Parakeet TDT 0.6B v3", { input: ["audio"], stt_path: "transcriptions", price_label: "$0.09/h audio" }),
    row("google/gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite", { input: ["text", "audio"], stt_path: "chat_audio", context_length: 1048576, price_label: "≈ $0.035/h + output", badges: ["ZDR", "chat model, may paraphrase"] }),
    row("mistralai/voxtral-small", "Voxtral Small (logs)", { input: ["audio"], stt_path: "chat_audio", zdr: false, price_label: "≈ $0.1/h + output", badges: ["chat model, may paraphrase"] }),
    row("meta-llama/llama-3.3-70b-instruct", "Llama 3.3 70B Instruct", { context_length: 131072, recommended: true, price_label: "≈ $0.0007 per dictation", badges: ["ZDR", "fast"] }),
    row("meta-llama/llama-3.1-8b-instruct", "Llama 3.1 8B Instruct", { context_length: 131072, recommended: true, price_label: "≈ $0.00006 per dictation", badges: ["ZDR", "fast"] }),
    row("openai/gpt-oss-20b", "gpt-oss-20b", { context_length: 131072, reasoning_mandatory: true, price_label: "≈ $0.0001 per dictation", badges: ["ZDR", "fast", "reasoning"] }),
    row("meta-llama/llama-3.3-8b-instruct:free", "Llama 3.3 8B (free)", { context_length: 128000, free: true, zdr: false, price_label: "free", badges: ["free (20/min, may log)"], allowed_by_account: false }),
  ];
  let catStatus = { source: "Baked", fetched_at_ms: 0, snapshot_date: "2026-10-24", models: catalog.length, stt: 5, cleanup: 4 };
  let or = {
    privacy: { zdr: true, data_collection: "deny" },
    stt: { allow: false, consent: false, consent_at_ms: 0, model: "openai/whisper-large-v3-turbo", stt_path: "auto", language: "", send_dictionary: true },
    cleanup: { allow: false, consent: false, consent_at_ms: 0, order: ["cloud", "local"], model: "meta-llama/llama-3.3-70b-instruct", sort: "latency", timeout_ms: 2500, send_surrounding_text: false, send_screen_words: false, send_app_name: false },
    budget: null,
    notice: "Cloud speech now runs only through OpenRouter. Your Groq provider was removed; your OpenRouter key was kept. Cloud speech was turned off: turn it on again in this tab.",
  };
  const usage = () => ({
    month: "2026-10", speech: "$0.0234", cleanup: "$0.0140", total: "$0.0374", budget: or.budget, budget_reached: false,
    rows: [
      { purpose: "stt", label: "Speech (openai/whisper-large-v3-turbo)", requests: 42, minutes: 5.2, cost: "$0.0234" },
      { purpose: "cleanup", label: "Cleanup (meta-llama/llama-3.3-70b-instruct)", requests: 20, minutes: 0, cost: "$0.0140" },
    ],
  });
  const cleanupStatus = () => ({
    supported: true, enabled: or.cleanup.allow && or.cleanup.consent && !!orKey, consent: or.cleanup.consent,
    blocked: or.cleanup.allow && !orKey ? "cloud_cleanup_not_configured" : null,
    order: or.cleanup.order, circuit: { open: false, reason: null, retry_in_ms: null, until_config_change: false },
    model: or.cleanup.model, timeout_ms: or.cleanup.timeout_ms,
    last: { engine: "cloud", model: or.cleanup.model, cost_usd: 0.0007, ttft_ms: 310, latency_ms: 640, generation_id: "gen-mock" },
    month_cost_usd: 0.014,
  });
  const info = (id) => catalog.find((r) => r.id === id) ?? null;
  const orView = () => ({
    supported: true, key: keyState(), privacy: { ...or.privacy }, stt: { ...or.stt },
    cleanup: { ...or.cleanup, status: cleanupStatus() }, budget: or.budget, usage: usage(), catalog: { ...catStatus },
    notice: or.notice,
    warnings: [!orKey && (or.stt.allow || or.cleanup.allow) ? "OpenRouter is on but no key is saved." : "", or.privacy.zdr ? "" : "Zero retention is off: providers may keep your audio/text."].filter(Boolean),
    stt_model_info: info(or.stt.model), cleanup_model_info: info(or.cleanup.model),
  });
  const validId = (m) => /^[a-z0-9._~/-]+(:[a-z]+)*$/.test(m) && m.length <= 128 && m.includes("/");
  // Rough stand-in for the backend's DevForm::validate (the real rules are Rust).
  const devCheck = (f) => {
    const errors = [];
    const h = f.host.trim();
    if (h && (/[\s/]|:\/\//.test(h) || (/^[\d.]+$/.test(h) && !/^(\d{1,3}\.){3}\d{1,3}$/.test(h)) || /^[^\[:]*:\d+$/.test(h)))
      errors.push({ field: "host", message: "Not a valid host" });
    if (!(f.port >= 1 && f.port <= 65535)) errors.push({ field: "port", message: "Port must be a whole number from 1 to 65535" });
    if (f.token && f.token.length < 16) errors.push({ field: "token", message: "Token must be at least 16 characters (or empty)" });
    const warnings = [];
    if (!f.allow_server && !f.allow_client) warnings.push("Dictation disabled — enable server or client in Developer settings");
    else if (f.allow_server && !f.tls && h) warnings.push("Unencrypted: OK only on your LAN or over an encrypted VPN.");
    return { errors, warnings, tls_forced: /\.(com|org|net)$/.test(h) && !h.endsWith(".ts.net") };
  };
  const health = {
    ok: true, engine: "remote", error: null, message: null, server: "100.64.0.10:7313", rtt_ms: { min: 3, median: 4, max: 6 },
    server_version: "flow-server 0.1.0", server_core: "0.1.0", postprocess_rev: "2026-10-21.1", postprocess_match: true,
    proto: 1, asr_backend: "parakeet", asr_model: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", llm: false, workers: 1, busy: 0,
  };
  const route = () => ({
    last: { engine: "local", fallback: "connect_failed", server: "100.64.0.10:7313", rtt_ms: null, queue_ms: null,
            server_core: null, asr_ms: 812, total_ms: 1630, status: "FORMATTED", empty_reason: null, error: null, at_ms: Date.now() - 5000,
            client_ms: 1650, wait_ms: 0, segments: 2, pre_asr_ms: 300, llm: "AI cleaned", llm_engine: "cloud",
            llm_model: "meta-llama/llama-3.3-70b-instruct", llm_cost_usd: 0.0007,
            llm_row: "AI cleaned · OpenRouter · meta-llama/llama-3.3-70b-instruct · $0.0007", row: null, notice: null },
    circuit: { open: true, reason: "connect_failed", retry_in_ms: 4200, until_config_change: false },
    health,
    cloud_circuit: { open: false, reason: null, retry_in_ms: null, until_config_change: false },
  });
  const devView = () => ({
    form: structuredClone(dev), caps: { local: true, remote: true, cloud: true },
    capabilities: { local: true, remote: true, asr_engines: ["whisper", "parakeet"], llm: true, proto: 1, core_version: "0.1.0" },
    visible: true, version: "0.1.0", build: "abc1234-dirty", full: "0.1.0+abc1234.dirty", check: devCheck(dev), route: route(),
  });
  let insights = {
    total_words: 12840, week_words: 2315, dictations: 412, total_ms: 5_400_000, avg_wpm: 142.7,
    current_streak: 4, longest_streak: 11, typing_wpm: 40, saved_minutes: 231,
    top_apps: [{ app: "slack.exe", words: 6120 }, { app: "code.exe", words: 3011 }, { app: "", words: 90 }],
    last7: [120, 0, 540, 300, 780, 210, 365].map((words, i) => ({ date: `2026-10-${22 + i}`, words })),
    since: "2026-08-02",
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
  let recentApps = ["slack.exe", "notepad.exe", "whatsapp.exe", "acme-crm.exe"];
  const defaults = [
    { key: "olk", exe: "olk.exe", name: "Outlook", default_bucket: "email" },
    { key: "slack", exe: "slack.exe", name: "Slack", default_bucket: "work" },
    { key: "telegram", exe: "telegram.exe", name: "Telegram", default_bucket: "personal" },
    { key: "whatsapp", exe: "whatsapp.exe", name: "WhatsApp", default_bucket: "personal" },
  ];
  let suggestions = [{ word: "Wispr Flow", heard: "whisper flow", app: "slack.exe", ts: now, count: 2 }];
  const listeners = {};
  const debugLog = { enabled: false, count: 12 };
  const debugView = () => ({ enabled: debugLog.enabled, count: debugLog.count, path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\debug\\dictations.jsonl", stats: { written: 0, dropped: 0, write_errors: 0 } });
  const cmds = {
    get_prefs: () => structuredClone(prefs),
    set_prefs: ({ prefs: p }) => { prefs = structuredClone(p); },
    list_microphones: () => ["USB Mic", "Realtek"],
    get_model_status: () => ({
      status: { state: "ready", path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\ggml-base.bin", load_ms: 312 },
      default_path: "C:\\…\\ggml-base.bin", default_parakeet_dir: "C:\\…\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", models_dir: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models",
      llm: { state: "plugin_missing", path: "C:\\…\\qwen.gguf", message: "llm plugin not found (C:\\Program Files\\Murmur\\flow_llm.dll: not found)" },
      default_llm_path: "C:\\…\\qwen2.5-0.5b-instruct-q4_k_m.gguf", llm_plugin: "flow_llm.dll",
    }),
    history_list: ({ query }) => history.filter((h) => !query || JSON.stringify(h).toLowerCase().includes(query.toLowerCase())),
    history_delete: ({ id }) => { history = history.filter((h) => h.id !== id); return true; },
    history_clear: () => { const n = history.length; history = []; return n; },
    history_copy: () => {}, history_repaste: () => {}, clipboard_copy: () => {},
    insights_get: () => structuredClone(insights),
    insights_reset: () => {
      insights = { ...insights, total_words: 0, week_words: 0, dictations: 0, total_ms: 0, avg_wpm: null,
        current_streak: 0, longest_streak: 0, saved_minutes: 0, top_apps: [], since: null,
        last7: insights.last7.map((d) => ({ ...d, words: 0 })) };
    },
    suggestions_list: () => suggestions,
    app_style_choices: () => ({
      recent: recentApps.map((exe) => {
        const key = exe.replace(/\.exe$/, "");
        const d = defaults.find((x) => x.key === key);
        return { key, exe, name: d ? d.name : key === "notepad" ? "Notepad" : null, default_bucket: d ? d.default_bucket : "other" };
      }),
      defaults,
    }),
    recent_apps_clear: () => { recentApps = []; },
    suggestion_accept: ({ word }) => {
      suggestions = suggestions.filter((s) => s.word !== word);
      prefs.dictionary.push({ word, replacement: null, isSnippet: false });
      return structuredClone(prefs);
    },
    suggestion_dismiss: ({ word }) => { suggestions = suggestions.filter((s) => s.word !== word); },
    dictionary_import: ({ entries, json, replace }) => ({ entries: (replace ? [] : entries).concat(JSON.parse(json)), added: 1, updated: 0, skipped: 0 }),
    dictionary_export: () => "C:\\Users\\u\\Downloads\\flow-dictionary.json",
    dictionary_export_text: ({ entries }) => JSON.stringify(entries, null, 2),
    dev_get: devView,
    dev_validate: ({ form }) => devCheck(form),
    dev_set: ({ form }) => { const c = devCheck(form); if (c.errors.length) throw c.errors; dev = structuredClone(form); return devView(); },
    dev_test_connection: () => health,
    dev_test_session: () => ({ status: "EMPTY", empty_reason: "no_speech", error: null, engine: "remote", fallback: null, total_ms: 212 }),
    dev_route_status: route,
    dev_reset_circuit: route,
    dev_reset_defaults: devView,
    dev_unlock: devView,
    dev_cloud_set_key: ({ key }) => { orKey = key.trim(); return keyState(); },
    dev_cloud_clear_key: () => { orKey = ""; return keyState(); },
    dev_cloud_usage: usage,
    dev_cloud_reset_usage: usage,
    dev_or_get: orView,
    dev_or_ack_notice: () => { or.notice = null; return orView(); },
    dev_or_key_info: () => orKey
      ? { label: "flow-clone", limit: null, limit_remaining: null, limit_reset: null, usage: 1.23, usage_daily: 0.02, usage_weekly: 0.2, usage_monthly: 0.84, is_free_tier: false, free_requests_remaining: null, checked_at_ms: Date.now(), warn_no_limit: true }
      : { error: "cloud_not_configured" },
    dev_or_set_privacy: ({ zdr, dataCollection }) => { or.privacy = { zdr, data_collection: dataCollection }; return orView(); },
    dev_or_set_stt: ({ form }) => {
      if (form.allow && !or.stt.consent && !form.consent) throw "consent required before turning on cloud speech";
      if (form.consent) { or.stt.consent = true; or.stt.consent_at_ms = Date.now(); }
      or.stt = { ...or.stt, allow: form.allow, language: form.language, send_dictionary: form.send_dictionary, stt_path: form.stt_path };
      return orView();
    },
    dev_or_set_cleanup: ({ form }) => {
      if (form.allow && !or.cleanup.consent && !form.consent) throw "consent required before turning on cloud cleanup";
      if (form.consent) { or.cleanup.consent = true; or.cleanup.consent_at_ms = Date.now(); }
      const { consent, ...rest } = form;
      or.cleanup = { ...or.cleanup, ...rest, timeout_ms: Math.min(8000, Math.max(800, form.timeout_ms)) };
      return orView();
    },
    dev_or_set_budget: ({ budget }) => { or.budget = budget; return orView(); },
    dev_or_set_model: ({ purpose, model }) => {
      if (!validId(model)) throw `bad_config: "${model}" is not an OpenRouter model id`;
      or[purpose].model = model;
      return orView();
    },
    dev_or_catalog: ({ purpose, query, filters }) => catalog.filter((r) =>
      (purpose === "stt" ? r.stt_path !== null : r.input.includes("text") && r.stt_path === null) &&
      (!query || (r.id + r.name).toLowerCase().includes(query.toLowerCase())) &&
      (!filters.zdr_only || r.zdr) && (!filters.fast_only || r.badges.includes("fast")) &&
      (!filters.free_only || r.free) && (!filters.stt_path || r.stt_path === filters.stt_path)),
    dev_or_catalog_status: () => ({ ...catStatus }),
    dev_or_refresh_catalog: async ({ force }) => {
      await new Promise((r) => setTimeout(r, 150));
      catStatus = { ...catStatus, source: "Live", fetched_at_ms: Date.now(), snapshot_date: null };
      window.__refreshes = (window.__refreshes || 0) + 1;
      return force && window.__failRefresh ? { ...catStatus, source: "Cache", error: "connect_failed" } : { ...catStatus };
    },
    dev_or_test_stt: ({ key }) => (key || orKey)
      ? { ok: true, engine: "cloud", provider: "openrouter", model: or.stt.model, transcript: "And so, my fellow Americans", latency_ms: 640, cost_usd: 0.00006 }
      : { ok: false, engine: "cloud", error: "cloud_not_configured", message: "no API key" },
    dev_or_test_cleanup: () => orKey
      ? { ok: true, text: "Let's meet tomorrow at 3 pm.", error: null, message: null, latency_ms: 580, cost_usd: 0.0007, model: or.cleanup.model }
      : { ok: false, text: null, error: "cloud_cleanup_not_configured", message: "no API key", model: or.cleanup.model },
    dev_or_cleanup_status: cleanupStatus,
    debug_log_status: debugView,
    debug_log_set: ({ enabled }) => { debugLog.enabled = enabled; return debugView(); },
    debug_log_clear: () => { debugLog.count = 0; return debugView(); },
    debug_log_open_folder: () => null,
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
// Old tab names -> page ids (the app shell's data-page values).
const OLD_TABS = { general: "settings-general", ai: "settings-cleanup", history: "home", developer: "settings-developer", openrouter: "settings-openrouter" };
window.__goPage = (id) => document.querySelector(`[data-page="${OLD_TABS[id] ?? id}"]`)?.click();
// ?tab=home / ?tab=settings-speech etc.: open that page once the page has booted (screenshots).
window.addEventListener("load", () => {
  const tab = new URLSearchParams(location.search).get("tab");
  if (tab) setTimeout(() => window.__goPage(tab), 50);
  const undo = new URLSearchParams(location.search).get("undo");
  if (undo) setTimeout(() => [...document.querySelectorAll("button")].find((b) => b.textContent.startsWith("Undo"))?.click(), 300);
});
window.addEventListener("error", (e) => console.error("PAGE ERROR", e.message));
window.addEventListener("unhandledrejection", (e) => console.error("UNHANDLED", String(e.reason)));
// ?drive=or: click through the OpenRouter tab (notice, key, consents, tests,
// picker, privacy, budget) and log what the page shows.
window.addEventListener("load", () => {
  if (new URLSearchParams(location.search).get("drive") !== "or") return;
  const $ = (id) => document.getElementById(id);
  // Results go into <pre id="__drive"> so `--dump-dom` shows them.
  const out = document.createElement("pre");
  out.id = "__drive";
  document.body.append(out);
  const log = (...a) => { console.log(...a); out.textContent += a.join(" ") + "\n"; };
  window.addEventListener("error", (e) => log("PAGE ERROR", e.message));
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const calls = (c) => window.__mockCalls.filter((x) => x[0] === c);
  (async () => {
    await wait(300);
    log("BOOT tab hidden:", $("tab_openrouter").hidden, "| badge hidden:", $("or_badge").hidden, "| refreshes:", window.__refreshes || 0);
    log("PRIVACY LINE", $("privacy_line").textContent);
    window.__goPage("settings-openrouter");
    await wait(300);
    log("OPEN refreshes:", window.__refreshes || 0, "| notice:", $("or_notice").textContent.slice(0, 40));
    $("or_notice_ok").click();
    await wait(100);
    log("NOTICE hidden:", $("or_notice_box").hidden, "| badge hidden:", $("or_badge").hidden);
    $("or_key").value = "sk-or-v1-PROBE-0123456789abcd";
    $("or_key_save").click();
    await wait(150);
    log("KEY", $("or_key_state").textContent, "| field now:", JSON.stringify($("or_key").value));
    $("or_key_check").click();
    await wait(100);
    log("KEYINFO", $("or_key_info").textContent, "| warn shown:", !$("or_key_warn").hidden);
    $("or_stt_allow").click();
    await wait(100);
    log("STT DIALOG open:", $("or_stt_consent_dlg").open);
    $("or_stt_consent_cancel").click();
    await wait(100);
    log("STT after cancel checked:", $("or_stt_allow").checked, "| set_stt calls:", calls("dev_or_set_stt").length);
    $("or_stt_allow").click();
    await wait(100);
    $("or_stt_consent_ok").click();
    await wait(400);
    log("STT on:", $("or_stt_allow").checked, "| consent:", $("or_stt_consent").textContent, "| form:", JSON.stringify(calls("dev_or_set_stt").at(-1)[1]));
    log("REFRESH after enable:", JSON.stringify(calls("dev_or_refresh_catalog").map((c) => c[1])));
    log("MODEL", $("or_stt_model_name").textContent, "|", $("or_stt_model_badges").textContent);
    $("or_stt_test").click();
    await wait(100);
    log("STT TEST", $("or_stt_test_out").textContent);
    $("or_cleanup_allow").click();
    await wait(100);
    log("CLEANUP DIALOG open:", $("or_cleanup_consent_dlg").open);
    $("or_cleanup_consent_ok").click();
    await wait(400);
    log("CLEANUP form:", JSON.stringify(calls("dev_or_set_cleanup").at(-1)[1]));
    log("CLEANUP STATE", $("or_cleanup_state").textContent);
    $("or_cleanup_test").click();
    await wait(100);
    log("CLEANUP TEST", $("or_cleanup_test_out").textContent);
    $("or_stt_change").click();
    await wait(200);
    log("PICKER open:", $("or_picker").open, "|", $("or_cat_status").textContent, "| rows:", $("or_picker_list").children.length);
    $("or_chip_zdr").click();
    await wait(100);
    log("ZDR off rows:", $("or_picker_list").children.length);
    $("or_chip_chat_audio").click();
    await wait(100);
    log("chat audio rows:", [...$("or_picker_list").querySelectorAll("code")].map((c) => c.textContent).join(","));
    $("or_picker_list").querySelector("button").click();
    await wait(100);
    log("PARAPHRASE confirm open:", $("or_confirm").open, "|", $("or_confirm_body").textContent.slice(0, 40));
    $("or_confirm_ok").click();
    await wait(200);
    log("PICKED", $("or_stt_model_id").textContent, "| picker open:", $("or_picker").open);
    $("or_cleanup_change").click();
    await wait(200);
    $("or_picker_query").value = "8b";
    $("or_picker_query").dispatchEvent(new Event("input"));
    await wait(350);
    log("SEARCH 8b rows:", $("or_picker_list").textContent);
    $("or_picker_custom").value = "Not A Model";
    $("or_picker_use").click();
    await wait(100);
    log("CUSTOM BAD", $("or_picker_status").textContent);
    $("or_cat_refresh").click();
    log("REFRESHING", $("or_cat_status").textContent);
    await wait(300);
    log("REFRESHED", $("or_cat_status").textContent);
    $("or_picker_close").click();
    $("or_zdr").click();
    await wait(100);
    log("ZDR CONFIRM open:", $("or_confirm").open, "|", $("or_confirm_body").textContent);
    $("or_confirm_cancel").click();
    await wait(100);
    log("ZDR still:", $("or_zdr").checked, "| privacy calls:", calls("dev_or_set_privacy").length);
    $("or_budget").value = "5";
    $("or_budget").dispatchEvent(new Event("change"));
    await wait(100);
    log("USAGE", $("or_usage_total").textContent, "|", $("or_usage_rows").textContent);
    window.__goPage("settings-cleanup");
    await wait(100);
    log("AI ENGINE", $("ai_engine_line").textContent);
    window.__goPage("settings-speech");
    await wait(200);
    log("ROUTES", [...$("dev_routes").querySelectorAll("[data-route]")].map((r) => r.dataset.route + (r.classList.contains("is-off") ? "(off)" : "")).join(" > "));
    window.__goPage("settings-developer");
    await wait(200);
    log("LAST", $("dev_last").textContent);
    window.__goPage("settings-server");
    await wait(100);
    $("dev_save").click();
    await wait(200);
    log("DEV_SET keys:", Object.keys(calls("dev_set").at(-1)?.[1].form ?? {}).join(","));
    log("LEAK", JSON.stringify(window.__mockCalls.filter((c) => c[0] !== "dev_cloud_set_key")).includes("PROBE"));
    log("TOTAL refreshes:", window.__refreshes);
    log("DRIVE DONE");
  })().catch((e) => log("DRIVE FAILED", String(e)));
});
