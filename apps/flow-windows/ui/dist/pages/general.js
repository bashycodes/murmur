// General tab: shortcuts, audio, speech model, transcription, insertion,
// app. Part of the shared Save form (with AI cleanup and Style); owns the
// prefs fields it shows and nothing else.
import { $, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { views } from "../lib/views.js";
import { versionTapped } from "./developer.js";
const TEMPLATE = `
<section class="panel" data-panel="general">
  <fieldset>
    <legend>Shortcuts</legend>
    <label>Push to talk (hold) <input id="ptt" placeholder="Ctrl+Win" /></label>
    <label>Hands-free (toggle) <input id="hands_free" placeholder="Ctrl+Win+Space" /></label>
    <label title="Hold, say an instruction, release: rewrites the selected text (or writes new text at the cursor) with the AI cleanup model">Command Mode (hold) <input id="command" placeholder="Ctrl+Win+Alt" /></label>
    <label>Cancel <input id="cancel" placeholder="Esc" /></label>
    <label>Paste last transcript <input id="paste_last" placeholder="Shift+Alt+Z" /></label>
    <label>Copy last transcript <input id="copy_last" placeholder="Shift+Alt+X" /></label>
    <label>Open Scratchpad <input id="scratchpad" placeholder="none (e.g. Ctrl+Alt+N)" /></label>
    <label>New note + dictate <input id="new_note" placeholder="none (e.g. Ctrl+Alt+M)" /></label>
    <p class="hint">Keys: Ctrl, Win, Alt, Shift, Space, Esc, A–Z, 0–9, F1–F24. Ctrl/Alt/Shift match either side; Left Alt, Right Alt (also Left/Right Ctrl, Shift) match one side only, e.g. push to talk = Right Alt. Max 3 keys, at least one modifier.</p>
  </fieldset>

  <fieldset>
    <legend>Audio</legend>
    <label>Microphone <select id="microphone"></select></label>
    <label class="row"><input type="checkbox" id="enable_sounds" /> Dictation sounds</label>
    <label class="row"><input type="checkbox" id="mute_audio_while_dictating" /> Mute other audio while dictating</label>
    <label>Max dictation length (minutes) <input id="max_dictation_minutes" type="number" min="1" max="60" /></label>
    <label>Hands-free: stop after this many seconds of silence (0 = never) <input id="hands_free_silence_s" type="number" min="0" max="10" step="0.1" /></label>
  </fieldset>

  <fieldset data-needs="local">
    <legend>Speech model</legend>
    <p id="model_status" class="hint">Checking…</p>
    <label>Engine
      <select id="asr_engine">
        <option value="whisper">Whisper (any language)</option>
        <option value="parakeet">Parakeet (English only, faster)</option>
      </select>
    </label>
    <label data-engine="whisper">Model file (empty = default) <input id="model_path" spellcheck="false" /></label>
    <label data-engine="parakeet">Parakeet model folder (empty = default) <input id="parakeet_model_dir" spellcheck="false" /></label>
    <p class="hint">Whisper: a whisper.cpp ggml model, e.g. <code>ggml-base.bin</code> (default) or <code>ggml-small-q5_1.bin</code> for better accuracy. Parakeet: the <code>sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8</code> folder (needs a build with the <code>parakeet</code> feature). Models are never downloaded automatically: put them in <code id="models_dir"></code>.</p>
  </fieldset>

  <fieldset>
    <legend>Transcription</legend>
    <label>Languages (comma-separated, empty = auto) <input id="languages" placeholder="en, de" /></label>
    <label class="row"><input type="checkbox" id="spoken_cleanup" /> Auto-format punctuation, quotes, fillers and corrections</label>
    <p class="hint">"comma", "new line", "quote … unquote", "in parentheses", "um", "scratch that", "$5", "3pm" — handled on this PC with or without the AI model. Off = exactly what you said (same as cleanup level Verbatim).</p>
    <label class="row"><input type="checkbox" id="use_ax_context" /> Use text around the cursor as context (stays local)</label>
    <label class="row"><input type="checkbox" id="live_preview" /> Show live transcript while dictating</label>
    <label class="row"><input type="checkbox" id="learn_from_edits" /> Suggest dictionary words from my corrections</label>
    <p class="hint">After a paste the field is re-read for about 45 s. Words you fix show up under Dictionary → Suggestions; nothing is added without your OK.</p>
  </fieldset>

  <fieldset>
    <legend>Insertion</legend>
    <label>Method
      <select id="paste_method">
        <option value="ctrl_v">Clipboard + Ctrl+V</option>
        <option value="shift_insert">Clipboard + Shift+Insert</option>
        <option value="type">Type characters</option>
      </select>
    </label>
    <label class="row"><input type="checkbox" id="restore_clipboard" /> Restore my clipboard afterwards</label>
  </fieldset>

  <fieldset>
    <legend>App</legend>
    <label class="row"><input type="checkbox" id="show_flow_bar_always" /> Show the Flow bar when idle</label>
    <label class="row"><input type="checkbox" id="launch_at_login" /> Launch at login</label>
    <p class="hint version" id="version_row" title="Version">Murmur</p>
  </fieldset>
</section>`;
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
    $("model_path").value = prefs.model_path ?? "";
    $("asr_engine").value = prefs.asr_engine;
    $("parakeet_model_dir").value = prefs.parakeet_model_dir ?? "";
    showEngineFields();
    $("ptt").value = prefs.shortcuts.ptt;
    $("hands_free").value = prefs.shortcuts.hands_free;
    $("command").value = prefs.shortcuts.command;
    $("cancel").value = prefs.shortcuts.cancel;
    $("paste_last").value = prefs.shortcuts.paste_last;
    $("copy_last").value = prefs.shortcuts.copy_last;
    $("scratchpad").value = prefs.shortcuts.scratchpad ?? "";
    $("new_note").value = prefs.shortcuts.new_note ?? "";
    $("microphone").value = prefs.microphone ?? "";
    for (const f of BOOL_FIELDS)
        $(f).checked = Boolean(prefs[f]);
    $("paste_method").value = prefs.paste_method;
    $("languages").value = prefs.languages.join(", ");
    $("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
    $("hands_free_silence_s").value =
        prefs.hands_free_silence_ms > 0 ? String(prefs.hands_free_silence_ms / 1000) : "0";
}
/** The prefs fields this tab owns, as shown in the form. */
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
export const generalPage = {
    id: "general",
    mount(root) {
        root.append(html(TEMPLATE));
        $("asr_engine").addEventListener("change", showEngineFields);
        $("version_row").addEventListener("click", versionTapped);
        views.onDev((v) => {
            $("version_row").textContent = `Murmur ${v.version} · ${v.build}`;
        });
        void onEvent("model:status", (e) => showModel(e.payload));
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
