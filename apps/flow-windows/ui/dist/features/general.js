// The everyday prefs fields on the General, System, Speech to text and AI
// cleanup settings pages (shortcuts, voice, sounds, pasting, the speech
// model on this PC, formatting): fill them from the prefs store and
// collect them for a save. Owns these fields and nothing else.
import { $, setValue } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
const BOOL_FIELDS = [
    "enable_sounds",
    "mute_audio_while_dictating",
    "launch_at_login",
    "show_flow_bar_always",
    "use_ax_context",
    "restore_clipboard",
    "live_preview",
    "learn_from_edits",
    "spoken_cleanup",
];
function showModel(st) {
    const e = $("model_status");
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
function showEngineFields() {
    const engine = $("asr_engine").value;
    for (const l of document.querySelectorAll("[data-engine]")) {
        l.hidden = l.dataset.engine !== engine;
    }
}
function fill(prefs) {
    setValue("model_path", prefs.model_path ?? "");
    $("asr_engine").value = prefs.asr_engine;
    setValue("parakeet_model_dir", prefs.parakeet_model_dir ?? "");
    showEngineFields();
    setValue("ptt", prefs.shortcuts.ptt);
    setValue("hands_free", prefs.shortcuts.hands_free);
    setValue("command", prefs.shortcuts.command);
    setValue("cancel", prefs.shortcuts.cancel);
    setValue("paste_last", prefs.shortcuts.paste_last);
    setValue("copy_last", prefs.shortcuts.copy_last);
    setValue("scratchpad", prefs.shortcuts.scratchpad ?? "");
    setValue("new_note", prefs.shortcuts.new_note ?? "");
    $("microphone").value = prefs.microphone ?? "";
    for (const f of BOOL_FIELDS)
        $(f).checked = Boolean(prefs[f]);
    $("paste_method").value = prefs.paste_method;
    setValue("languages", prefs.languages.join(", "));
    setValue("max_dictation_minutes", String(prefs.max_dictation_minutes));
    setValue("hands_free_silence_s", prefs.hands_free_silence_ms > 0 ? String(prefs.hands_free_silence_ms / 1000) : "0");
}
/** The prefs fields this module owns, as the pages show them. */
export function collectGeneral() {
    const next = {
        shortcuts: {
            ptt: $("ptt").value.trim(),
            hands_free: $("hands_free").value.trim(),
            command: $("command").value.trim(),
            cancel: $("cancel").value.trim(),
            paste_last: $("paste_last").value.trim(),
            copy_last: $("copy_last").value.trim(),
            scratchpad: $("scratchpad").value.trim(),
            new_note: $("new_note").value.trim(),
        },
    };
    next.model_path = $("model_path").value.trim() || null;
    next.asr_engine = $("asr_engine").value;
    next.parakeet_model_dir = $("parakeet_model_dir").value.trim() || null;
    next.microphone = $("microphone").value || null;
    for (const f of BOOL_FIELDS)
        next[f] = $(f).checked;
    next.paste_method = $("paste_method").value;
    next.languages = $("languages")
        .value.split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    next.max_dictation_minutes = Math.max(1, parseInt($("max_dictation_minutes").value, 10) || 20);
    // Hands-free auto-stop (D19): 0 = off, else 0.3-10 s of silence.
    const hf = parseFloat($("hands_free_silence_s").value);
    next.hands_free_silence_ms =
        !Number.isFinite(hf) || hf <= 0 ? 0 : Math.round(Math.min(10, Math.max(0.3, hf)) * 1000);
    return next;
}
export const general = {
    init() {
        $("asr_engine").addEventListener("change", showEngineFields);
        void onEvent("model:status", (e) => showModel(e.payload));
        prefsStore.subscribe(fill);
    },
    async load() {
        const [mics, model] = await Promise.all([
            invoke("list_microphones"),
            invoke("get_model_status"),
        ]);
        showModel(model.status);
        $("models_dir").textContent = model.models_dir;
        $("model_path").placeholder = model.default_path;
        $("parakeet_model_dir").placeholder = model.default_parakeet_dir;
        const mic = $("microphone");
        mic.replaceChildren(new Option("System default", ""), ...mics.map((m) => new Option(m, m)));
        const p = prefsStore.get();
        if (p)
            fill(p);
    },
};
