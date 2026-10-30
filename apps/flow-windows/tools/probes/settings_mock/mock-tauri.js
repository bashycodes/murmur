// Browser stand-in for window.__TAURI__ so ui/dist/settings.html can be
// opened without Tauri (layout / runtime-error check). Probe only: the
// data below is invented and nothing here ships.
(() => {
  const now = Date.now();
  let prefs = {
    shortcuts: { ptt: "Ctrl+Win", hands_free: "Ctrl+Win+Space", command: "Ctrl+Win+Alt", cancel: "Esc", paste_last: "Alt+Shift+Z", copy_last: "Alt+Shift+X", scratchpad: "Ctrl+Alt+N" },
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
    asr_engine: "whisper", parakeet_model_dir: null, hebrew_model: "zipformer", spoken_cleanup: true,
    theme: "system", onboarded: true,
    transforms: {
      enabled: false, auto_apply: null, view_changes: "Win+Alt+O",
      items: [
        { transform: { id: "polish", name: "Polish", description: "Improve clarity and conciseness", instruction: "Rewrite the text so it reads clearly and concisely. Keep the meaning, the language and the voice." }, shortcut: "Win+Alt+1" },
        { transform: { id: "prompt_engineer", name: "Prompt Engineer", description: "Turns a rough ask into a clear prompt", instruction: "Turn the text into a clear, specific prompt for an AI assistant." }, shortcut: "Win+Alt+2" },
      ],
    },
  };
  // transforms::TransformPrefs::reset_builtins + Prefs::validate (the parts the page can trip).
  const BUILTIN_TRANSFORMS = structuredClone(prefs.transforms.items);
  const transformsCheck = (t) => {
    if (!t) return;
    if (t.items.length > 20) throw "at most 20 transforms";
    for (const i of t.items) {
      if (!i.transform.name.trim()) throw "every transform needs a name";
      if (!i.transform.instruction.trim()) throw `transform "${i.transform.name}" needs an instruction`;
      if (i.shortcut && !/^((Ctrl|Alt|Shift|Win)\+)+[A-Z0-9]$/i.test(i.shortcut)) throw `transform "${i.transform.name}" shortcut: "${i.shortcut}" is not a shortcut`;
    }
    const all = [...t.items.map((i) => i.shortcut), t.view_changes].filter(Boolean).map((c) => c.toLowerCase());
    const dup = all.find((c, n) => all.indexOf(c) !== n);
    if (dup) throw `shortcut ${dup} is used twice`;
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
    // 18 Monday-first weeks ending this week; null = after today.
    // Sparse early on, busier lately, lighter weekends, a 4-day streak to today.
    heatmap: Array.from({ length: 126 }, (_, i) => {
      const r = ((i * 7919) % 101) / 101, weekend = i % 7 >= 5, busy = i / 126;
      const active = i >= 120 || r < (weekend ? 0.3 : 0.5 + 0.45 * busy);
      return {
        date: new Date(Date.UTC(2026, 5, 29 + i)).toISOString().slice(0, 10),
        words: i > 123 ? null : i === 119 || !active ? 0 : Math.round((weekend ? 120 : 260 + 900 * busy) * (0.3 + r)),
      };
    }),
    routes_7d: { pc: 41, server: 63, cloud: 0, unknown: 2 },
    fixes: { ai: 297, dictionary: 269, total: 566 },
  };
  let history = [
    { id: 6, ts: now - 10e3, app: "slack.exe", url: null, raw: "Rewrite the text so it reads clearly and concisely.", command: true, transform: "Polish",
      original: "so basically i think we should probably ship it on friday if nothing breaks",
      formatted: "I think we should ship on Friday if nothing breaks.", pasted: "I think we should ship on Friday if nothing breaks.", edited: null,
      duration_ms: 0, words: 10, lang: null, route: "cloud",
      final_text: "I think we should ship on Friday if nothing breaks.", ai_edited: false },
    { id: 5, ts: now - 20e3, app: "chrome.exe", url: null, raw: "can we move the review to thursday i want one more pass over the parakeet benchmarks before we share them",
      formatted: "Can we move the review to Thursday? I want one more pass over the Parakeet benchmarks before we share them.",
      pasted: "Can we move the review to Thursday? I want one more pass over the Parakeet benchmarks before we share them.", edited: null,
      duration_ms: 6200, words: 20, lang: "en", route: "pc",
      final_text: "Can we move the review to Thursday? I want one more pass over the Parakeet benchmarks before we share them.", ai_edited: true },
    { id: 4, ts: now - 30 * 60e3, app: "code.exe", url: null, raw: "ship the hands free fix first then the clipboard restore",
      formatted: null, pasted: "Ship the hands-free fix first, then the clipboard restore.", edited: null,
      duration_ms: 3100, words: 10, lang: "en", route: "cloud", final_text: "Ship the hands-free fix first, then the clipboard restore.", ai_edited: false },
    { id: 3, ts: now - 60e3, app: "outlook.exe", url: null, raw: "um so i think we should uh meet tomorrow at three",
      formatted: "I think we should meet tomorrow at 3.", pasted: "I think we should meet tomorrow at 3.", edited: null,
      duration_ms: 4100, words: 8, lang: "en", route: "server", final_text: "I think we should meet tomorrow at 3.", ai_edited: true },
    { id: 2, ts: now - 3600e3, app: "slack.exe", url: null, raw: "i use whisper flow daily",
      formatted: null, pasted: "I use whisper flow daily", edited: "I use Wispr Flow daily",
      duration_ms: 2000, words: 5, lang: "en", route: "pc", final_text: "I use whisper flow daily", ai_edited: false },
    { id: 1, ts: now - 86400e3 * 2, app: "notepad.exe", url: null, raw: "hello world", formatted: null, pasted: null,
      edited: null, duration_ms: 900, words: 2, lang: "en", route: null, final_text: "hello world", ai_edited: false },
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
  // ?onboard=1: a fresh install (onboarded false, no server, no model yet):
  // first-run setup opens over the shell. Saving a model location
  // "loads" it: a path containing "missing" is not found, anything else is ready.
  const onboard = new URLSearchParams(location.search).get("onboard") === "1";
  let modelStatus = { state: "ready", path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\ggml-base.bin", load_ms: 312 };
  if (onboard) {
    prefs.onboarded = false;
    prefs.llm_enabled = false;
    dev = { ...dev, host: "", token: "", allow_server: true };
    or.notice = null;
    modelStatus = { state: "missing", path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\ggml-base.bin" };
  }
  const MODELS = "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\";
  const hf = (repo, rev, name, size, sha256) => ({ name, url: `https://huggingface.co/${repo}/resolve/${rev}/${name}`, size, sha256 });
  const PK = ["csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", "1ab9323565ddb038682214b292f588070a538ce2"];
  const WH = ["ggerganov/whisper.cpp", "5359861c739e955e79d9a303bcbc70fb988958b1"];
  const ZH = ["ivrit-ai/zipformer2-66m-streaming-he", "ee769a2a13b887f91572eec349458bfb0fe79e7a"];
  const PH = ["krut42/voice-parakeet-he-int8", "39de3b473bed626c7796c156d71adc66161ebfe1"];
  // H6: the language-ID model comes with each Hebrew model (models folder).
  const LID = hf("ggerganov/whisper.cpp", "5359861c739e955e79d9a303bcbc70fb988958b1", "ggml-tiny-q5_1.bin", 32152673, "8187…");
  const zh = (name, size, sha) => ({ ...hf(...ZH, name, size, sha), url: `https://huggingface.co/${ZH[0]}/resolve/${ZH[1]}/sherpa-onnx/${name}` });
  const dlCatalog = [
    { id: "parakeet-tdt-0.6b-v2-int8", title: "Parakeet TDT 0.6B", desc: "English · fastest", engine: "parakeet", recommended: true, size: 661190513,
      page: "https://huggingface.co/" + PK[0], dest: MODELS + "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", available: true,
      files: [hf(...PK, "encoder.int8.onnx", 652184296, "a32b…"), hf(...PK, "decoder.int8.onnx", 7257753, "b6bb…"), hf(...PK, "joiner.int8.onnx", 1739080, "7946…"), hf(...PK, "tokens.txt", 9384, "ec18…")] },
    { id: "whisper-base", title: "Whisper base", desc: "Many languages", engine: "whisper", recommended: false, size: 147951465,
      page: "https://huggingface.co/" + WH[0], dest: MODELS + "ggml-base.bin", available: true, files: [hf(...WH, "ggml-base.bin", 147951465, "60ed…")] },
    { id: "whisper-small-q5_1", title: "Whisper small", desc: "Many languages · more accurate, slower", engine: "whisper", recommended: false, size: 190085487,
      page: "https://huggingface.co/" + WH[0], dest: MODELS + "ggml-small-q5_1.bin", available: true, files: [hf(...WH, "ggml-small-q5_1.bin", 190085487, "ae85…")] },
    // D33 Hebrew route models (src/model_download.rs).
    { id: "zipformer-he-int8", title: "Zipformer Hebrew", desc: "Hebrew · recommended, small and fast", engine: "sherpa_online", recommended: true, size: 72316960 + 32152673,
      language: "he", license: "CC-BY-4.0 · Yakov Kolani / ivrit.ai",
      page: "https://huggingface.co/" + ZH[0], dest: MODELS + "zipformer2-66m-streaming-he-int8", available: true,
      files: [zh("encoder.int8.onnx", 70478964, "6c6f…"), zh("decoder.int8.onnx", 1049105, "feb0…"), zh("joiner.int8.onnx", 771805, "1d2c…"), zh("tokens.txt", 17086, "e4eb…"), LID] },
    { id: "parakeet-tdt-0.6b-v3-he-int8", title: "Parakeet Hebrew", desc: "Hebrew · larger alternative", engine: "parakeet", recommended: false, size: 672825290 + 32152673,
      language: "he", license: "CC-BY-4.0 · krut42, based on thewh1teagle/parakeet-tdt-0.6b-v3-he",
      page: "https://huggingface.co/" + PH[0], dest: MODELS + "parakeet-tdt-0.6b-v3-he-int8", available: true,
      files: [hf(...PH, "encoder.int8.onnx", 653215173, "7eaf…"), hf(...PH, "decoder.int8.onnx", 12494234, "3a1d…"), hf(...PH, "joiner.int8.onnx", 7008293, "056c…"), hf(...PH, "tokens.txt", 107590, "55a9…"), LID] },
  ];
  // The Hebrew model's load state (`model:hebrew`): null = none chosen.
  const heDir = { zipformer: "zipformer-he-int8", parakeet_he: "parakeet-tdt-0.6b-v3-he-int8" };
  let heStatus = null;
  const loadHebrew = () => {
    const c = dlCatalog.find((x) => x.id === heDir[prefs.hebrew_model]);
    if (!c) { heStatus = null; window.__emit?.("model:hebrew", heStatus); return; }
    heStatus = dlInstalled.has(c.id) ? { state: "ready", path: c.dest, load_ms: 410 } : { state: "missing", path: c.dest, engine: c.engine };
    window.__emit?.("model:hebrew", heStatus);
  };
  // ?he=nolid: Zipformer Hebrew downloaded before the language-ID model came with it.
  const heParam = new URLSearchParams(location.search).get("he");
  const dlInstalled = new Set(onboard ? [] : heParam === "nolid" ? ["whisper-base", "zipformer-he-int8"] : ["whisper-base"]);
  let lidPresent = false;
  const view = (c) => {
    const installed = dlInstalled.has(c.id);
    if (!c.language) return { ...c, installed, complete: installed, missing: installed ? 0 : c.size };
    const lid = lidPresent ? 0 : LID.size;
    return { ...c, installed, complete: installed && lidPresent, missing: installed ? lid : c.size - LID.size + lid };
  };
  loadHebrew();
  let dlRunning = null;
  const modelKey = (p) => [p.asr_engine, p.model_path, p.parakeet_model_dir].join("|");
  const debugView = () => ({ enabled: debugLog.enabled, count: debugLog.count, path: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\debug\\dictations.jsonl", stats: { written: 0, dropped: 0, write_errors: 0 } });
  // Notes as the app window sees them (list / create / pin / open only:
  // the text is the notes window's, mock-scratchpad.js). Opened ids land in
  // window.__opened.
  let notes = [
    { id: "scratchpad", title: "Quick note", preview: "Call the plumber back", words: 4, created_ms: now - 864e5 * 9, updated_ms: now - 3600e3, pinned: false, quick: true },
    { id: "note-1", title: "Release checklist", preview: "Bump VERSION, run check-build-matrix, tag, write the changelog entry and attach the installers", words: 24, created_ms: now - 864e5 * 3, updated_ms: now - 600e3, pinned: true, quick: false },
    { id: "note-2", title: "Reply to Sam", preview: "Thursday at 3 works. I'll bring the slides and the latest numbers.", words: 15, created_ms: now - 864e5 * 2, updated_ms: now - 864e5, pinned: false, quick: false },
    { id: "note-3", title: "Ideas", preview: "Per-app engine choice? Push-to-talk on the mouse side button.", words: 11, created_ms: now - 864e5 * 8, updated_ms: now - 864e5 * 8, pinned: false, quick: false },
  ];
  let noteSeq = 4;
  window.__opened = [];
  const noteOrder = (a, b) => b.pinned - a.pinned || b.updated_ms - a.updated_ms || (a.id < b.id ? -1 : 1);
  const cmds = {
    note_list: ({ query }) => {
      const q = (query || "").trim().toLowerCase();
      return notes.filter((n) => !q || (n.title + "\n" + n.preview).toLowerCase().includes(q)).sort(noteOrder).map((n) => ({ ...n }));
    },
    note_create: () => {
      const id = `note-${noteSeq++}`;
      notes.push({ id, title: "Untitled", preview: "", words: 0, created_ms: Date.now(), updated_ms: Date.now(), pinned: false, quick: false });
      return id;
    },
    note_set_pinned: ({ id, pinned }) => {
      const n = notes.find((x) => x.id === id);
      if (!n) throw `no note "${id}"`;
      n.pinned = pinned;
    },
    note_open: ({ id }) => {
      if (!/^[a-z0-9_-]{1,64}$/.test(id)) throw `"${id}" is not a note id`;
      window.__opened.push(id);
    },
    get_prefs: () => structuredClone(prefs),
    set_prefs: ({ prefs: p }) => {
      transformsCheck(p.transforms);
      const reload = modelKey(p) !== modelKey(prefs);
      const heChanged = p.hebrew_model !== prefs.hebrew_model;
      prefs = structuredClone(p);
      if (heChanged) setTimeout(loadHebrew, 100);
      if (reload) {
        const path = p.asr_engine === "parakeet"
          ? p.parakeet_model_dir || "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"
          : p.model_path || "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models\\ggml-base.bin";
        modelStatus = { state: "loading", path };
        setTimeout(() => {
          modelStatus = path.includes("missing") ? { state: "missing", path } : { state: "ready", path, load_ms: 900 };
          window.__emit("model:status", modelStatus);
        }, 150);
      }
    },
    list_microphones: () => ["USB Mic", "Realtek"],
    get_model_status: () => ({
      status: { ...modelStatus }, hebrew: heStatus && { ...heStatus },
      default_path: "C:\\…\\ggml-base.bin", default_parakeet_dir: "C:\\…\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", models_dir: "C:\\Users\\u\\AppData\\Roaming\\FlowClone\\models",
      llm: { state: "plugin_missing", path: "C:\\…\\qwen.gguf", message: "llm plugin not found (C:\\Program Files\\Murmur\\flow_llm.dll: not found)" },
      default_llm_path: "C:\\…\\qwen2.5-0.5b-instruct-q4_k_m.gguf", llm_plugin: "flow_llm.dll",
    }),
    transforms_reset: () => {
      const custom = prefs.transforms.items.filter((i) => !BUILTIN_TRANSFORMS.some((b) => b.transform.id === i.transform.id));
      const taken = custom.map((i) => i.shortcut).filter(Boolean);
      const items = structuredClone(BUILTIN_TRANSFORMS).map((i) => (taken.includes(i.shortcut) ? { ...i, shortcut: null } : i));
      prefs.transforms = { ...prefs.transforms, items: items.concat(custom) };
      if (!prefs.transforms.items.some((i) => i.transform.id === prefs.transforms.auto_apply)) prefs.transforms.auto_apply = null;
      return structuredClone(prefs);
    },
    // Shortcut recording (src/app.rs hotkey_capture_*): the hook's answer
    // is simulated with window.__capture("Right Alt") / __capture(null)
    // (Esc) / __capture(null, "timeout"). __noHook = the hook isn't running.
    hotkey_capture_start: () => {
      if (window.__noHook) throw "The keyboard hook is not running: type the shortcut instead.";
      window.__capturing = true;
      return null;
    },
    hotkey_capture_stop: () => { window.__capturing = false; return null; },
    transform_try: async ({ id, text }) => {
      const t = prefs.transforms.items.find((i) => i.transform.id === id);
      if (!t) throw `no transform "${id}"`;
      if (window.__failTransform) throw "Transforms need AI: turn on OpenRouter cleanup or a model on this PC.";
      await new Promise((r) => setTimeout(r, 50));
      return `${t.transform.name}: ${text.trim().replace(/^./, (c) => c.toUpperCase())}`;
    },
    history_list: ({ query }) => history.filter((h) => !query || JSON.stringify(h).toLowerCase().includes(query.toLowerCase())),
    history_delete: ({ id }) => { history = history.filter((h) => h.id !== id); return true; },
    history_clear: () => { const n = history.length; history = []; return n; },
    history_copy: () => {}, history_repaste: () => {}, clipboard_copy: () => {},
    insights_get: () => structuredClone(insights),
    insights_reset: () => {
      insights = { ...insights, total_words: 0, week_words: 0, dictations: 0, total_ms: 0, avg_wpm: null,
        current_streak: 0, longest_streak: 0, saved_minutes: 0, top_apps: [], since: null,
        last7: insights.last7.map((d) => ({ ...d, words: 0 })),
        heatmap: insights.heatmap.map((d) => ({ ...d, words: d.words === null ? null : 0 })),
        routes_7d: { pc: 0, server: 0, cloud: 0, unknown: 0 }, fixes: { ai: 0, dictionary: 0, total: 0 } };
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
      if (rest.order == null) delete rest.order; // null = keep (CleanupForm.order: Option)
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
    // Native picker: window.__pickResult overrides (null = the user cancelled).
    pick_path: ({ kind, filters }) => {
      if (window.__pickResult !== undefined) return window.__pickResult;
      if (kind === "folder") return "D:\\models\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";
      return (filters || []).some((f) => f.pattern.includes("gguf")) ? "D:\\models\\qwen2.5-1.5b-instruct-q4_k_m.gguf" : "D:\\models\\ggml-base.bin";
    },
    open_external: ({ url }) => {
      if (!/^https?:\/\/(github\.com\/bashycodes\/murmur|huggingface\.co\/|openrouter\.ai(\/|$))/.test(url)) throw new Error(`not an allowed link: ${url}`);
      return null;
    },
    // D31 downloader: a fake catalogue; a started download ticks to done in
    // about 1.5 s (?dlfail=1: fails its fingerprint check instead).
    model_catalog: () => dlCatalog.map(view),
    model_download_start: ({ id }) => {
      const c = dlCatalog.find((x) => x.id === id);
      if (!c) throw new Error(`unknown model ${id}`);
      if (dlRunning) throw new Error("another model is downloading; wait for it or cancel it");
      dlRunning = { id, received: 0, cancelled: false };
      const run = dlRunning;
      const fail = new URLSearchParams(location.search).get("dlfail") === "1";
      const ev = (state, extra = {}) => window.__emit("model:download", { id, received: run.received, total: c.size, state, ...extra });
      const tick = setInterval(() => {
        if (run.cancelled) {
          clearInterval(tick); dlRunning = null; run.received = 0; ev("cancelled"); return;
        }
        run.received = Math.min(c.size, run.received + Math.ceil(c.size / (window.__dlSteps || 6)));
        if (run.received < c.size) { ev("downloading"); return; }
        clearInterval(tick);
        ev("verifying");
        setTimeout(() => {
          dlRunning = null;
          if (fail) { run.received = 0; ev("error", { error: `${c.files[0].name} did not match its fingerprint, so it was deleted` }); return; }
          dlInstalled.add(id);
          const path = c.dest;
          if (c.language) {
            lidPresent = true;
            // A Hebrew model becomes the Hebrew route; the default stays.
            prefs = { ...prefs, hebrew_model: c.engine === "sherpa_online" ? "zipformer" : "parakeet_he" };
            ev("done", { path });
            loadHebrew();
            return;
          }
          prefs = c.engine === "parakeet" ? { ...prefs, asr_engine: "parakeet", parakeet_model_dir: path } : { ...prefs, asr_engine: "whisper", model_path: path };
          modelStatus = { state: "loading", path };
          ev("done", { path });
          window.__emit("model:status", modelStatus);
          setTimeout(() => { modelStatus = { state: "ready", path, load_ms: 900 }; window.__emit("model:status", modelStatus); }, 150);
        }, 150);
      }, window.__dlTickMs || 250);
      return null;
    },
    model_download_cancel: ({ id }) => {
      if (!dlRunning || dlRunning.id !== id) return false;
      dlRunning.cancelled = true;
      return true;
    },
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
  window.__capture = (chord, state) => {
    window.__capturing = false;
    window.__emit("hotkey:capture", chord ? { state: "done", chord } : { state: state || "cancelled", chord: null });
  };
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
