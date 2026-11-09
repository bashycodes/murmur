// The everyday prefs fields on the General, System, Speech to text and AI
// cleanup settings pages (shortcuts, voice, theme, sounds, pasting, the
// speech model on this PC, formatting): fill them from the prefs store and
// collect them for a save. Owns these fields and nothing else, plus the
// General page's keycap / Change editors (`.lw-edit`).
import { $, el, setStatus, setValue } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { captureChord, RECORDING_TEXT, stopCapture } from "../lib/keycapture.js";
import { prefsStore } from "../lib/prefs-store.js";
import { hebrewChosen, hebrewReady, models } from "./models.js";
import { setTheme } from "../lib/theme.js";
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
    $("hebrew_model").value = prefs.hebrew_model ?? "zipformer";
    $("model_source").value = prefs.model_source;
    showEngineFields();
    setValue("ptt", prefs.shortcuts.ptt);
    setValue("hands_free", prefs.shortcuts.hands_free);
    setValue("command", prefs.shortcuts.command);
    setValue("cancel", prefs.shortcuts.cancel);
    setValue("paste_last", prefs.shortcuts.paste_last);
    setValue("copy_last", prefs.shortcuts.copy_last);
    setValue("scratchpad", prefs.shortcuts.scratchpad ?? "");
    setValue("new_note", prefs.shortcuts.new_note ?? "");
    setValue("meeting", prefs.shortcuts.meeting ?? "");
    $("microphone").value = prefs.microphone ?? "";
    for (const f of BOOL_FIELDS)
        $(f).checked = Boolean(prefs[f]);
    $("paste_method").value = prefs.paste_method;
    setValue("languages", prefs.languages.join(", "));
    pick("max_dictation_minutes", String(prefs.max_dictation_minutes), (v) => `${v} minutes`);
    pick("hands_free_silence_s", prefs.hands_free_silence_ms > 0 ? String(prefs.hands_free_silence_ms / 1000) : "0", (v) => `${v} seconds`);
    pick("theme", typeof prefs.theme === "string" ? prefs.theme : "system", (v) => v);
    for (const e of document.querySelectorAll(".lw-edit"))
        showEdit(e);
    showLanguages(prefs);
    if (prefs.keyboard_language)
        refreshKeyboard();
}
/** Select `value`, adding it as an option first when the list lacks it
 *  (a value saved by hand or by an older version). */
function pick(id, value, label) {
    const s = $(id);
    if (![...s.options].some((o) => o.value === value))
        s.add(new Option(label(value), value));
    s.value = value;
}
// ----------------------------------------------- keycap / Change editors
/** Show the saved value: keycaps for a shortcut (or "Off"), nothing for languages. */
function showEdit(box) {
    const input = box.querySelector(".lw-edit-input");
    const view = box.querySelector(".lw-edit-view");
    if (!input || !view || box.classList.contains("is-recording"))
        return;
    const keys = input.value.split("+").map((k) => k.trim()).filter(Boolean);
    view.replaceChildren(...(keys.length ? keys.map((k) => el("kbd", "fl-kbd", k)) : [el("span", "lw-edit-off", "Off")]));
}
function setEditing(box, on) {
    const input = box.querySelector(".lw-edit-input");
    const view = box.querySelector(".lw-edit-view");
    const btn = box.querySelector(".lw-edit-btn");
    if (!input || !btn)
        return;
    box.classList.toggle("is-editing", on);
    input.hidden = !on;
    if (view)
        view.hidden = on;
    btn.textContent = on ? "Done" : "Change";
    if (box.dataset.edit !== "languages") {
        $("keys_help").hidden = !document.querySelector(".lw-edit.is-editing:not([data-edit=languages])");
    }
}
/** A shortcut's Change: "Press keys… Esc to cancel" until the user has
 *  held a chord and let go; it then saves like a typed one (the field's
 *  `change`). Cancel / Esc / 10 s / leaving the window keep the old one. */
function setRecording(box, on) {
    const view = box.querySelector(".lw-edit-view");
    const btn = box.querySelector(".lw-edit-btn");
    const type = box.querySelector(".lw-edit-type");
    box.classList.toggle("is-recording", on);
    if (btn)
        btn.textContent = on ? "Cancel" : "Change";
    if (type)
        type.hidden = !on;
    if (on)
        view?.replaceChildren(el("span", "lw-edit-rec", RECORDING_TEXT));
    else
        showEdit(box);
}
async function record(box, input) {
    setRecording(box, true);
    const r = await captureChord();
    // Cancel or Type instead already left recording.
    if (!box.classList.contains("is-recording"))
        return;
    setRecording(box, false);
    if ("chord" in r) {
        input.value = r.chord;
        showEdit(box);
        input.dispatchEvent(new Event("change", { bubbles: true }));
    }
    else if (r.message) {
        setStatus("status", r.message, false);
    }
}
/** Change records the keys (shortcuts) or swaps the keycaps for the text
 *  field (languages, or a shortcut's Type instead); Enter / Done / leaving
 *  the field saves (the field's own `change`), Esc puts the old value back. */
function wireEdit(box) {
    const input = box.querySelector(".lw-edit-input");
    const btn = box.querySelector(".lw-edit-btn");
    if (!input || !btn)
        return;
    let before = "";
    const startTyping = () => {
        before = input.value;
        setEditing(box, true);
        input.focus();
        input.select();
    };
    // Keep focus in the field so Done is not undone by the field's blur.
    btn.addEventListener("mousedown", (e) => {
        if (!input.hidden)
            e.preventDefault();
    });
    btn.addEventListener("click", () => {
        if (!input.hidden) {
            input.blur();
            return;
        }
        if (box.dataset.edit === "languages") {
            startTyping();
        }
        else if (box.classList.contains("is-recording")) {
            setRecording(box, false);
            stopCapture();
        }
        else {
            void record(box, input);
        }
    });
    box.querySelector(".lw-edit-type")?.addEventListener("click", () => {
        setRecording(box, false);
        stopCapture();
        startTyping();
    });
    input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
            e.preventDefault();
            input.blur();
        }
        else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation(); // not the Settings dialog's Esc
            input.value = before;
            input.blur();
        }
    });
    input.addEventListener("blur", () => {
        setEditing(box, false);
        showEdit(box);
        const p = prefsStore.get();
        if (box.dataset.edit === "languages" && p)
            showLanguages(p, input.value, false);
    });
}
/** English names of language codes ("he" → "Hebrew"); the codes without Intl.DisplayNames. */
function languageNames(codes) {
    try {
        const dn = new Intl.DisplayNames(["en"], { type: "language" });
        return codes.map((c) => {
            try {
                return dn.of(c) ?? c;
            }
            catch {
                return c;
            }
        });
    }
    catch {
        return codes;
    }
}
/** D37: the keyboard layout's language right now (`null` = unknown), as
 *  last read; re-read each time the languages are shown. */
let keyboardNow = null;
/** "English, German" (or "Detected automatically", or D37 "Keyboard
 *  language · now: Hebrew"), and the Parakeet note when the local engine
 *  only understands English. */
function showLanguages(prefs, typed, keyboard) {
    const codes = typed === undefined ? prefs.languages : typed.split(",").map((s) => s.trim()).filter(Boolean);
    const kb = keyboard ?? (typed === undefined && prefs.keyboard_language === true);
    const names = languageNames(codes);
    const desc = $("languages_desc");
    if (kb) {
        const now = keyboardNow ? languageNames([keyboardNow])[0] : "unknown";
        const fallback = names.length ? names.join(", ") : "detected automatically";
        desc.textContent = `Keyboard language · now: ${now}. Each dictation uses the language of the keyboard layout in the app you dictate into; an unknown layout is ${fallback}.`;
    }
    else {
        desc.textContent = names.length ? `${names.join(", ")}.` : "Detected automatically.";
    }
    // Hebrew has its own model (D33): the English-only note doesn't apply.
    const he = hebrewChosen(codes) || (kb && hebrewChosen(keyboardNow ? [keyboardNow] : []));
    const heModel = hebrewReady(prefs.hebrew_model);
    $("languages_parakeet").hidden = prefs.asr_engine !== "parakeet" || (he && heModel);
    // Only once the catalogue is known (it says what is downloaded).
    $("languages_hebrew").hidden = !he || heModel || models.catalog().length === 0;
    showLanguagePick(codes, kb);
}
/** Re-read the keyboard layout's language, then show the languages again. */
function refreshKeyboard() {
    invoke("keyboard_language")
        .then((k) => {
        if (k === keyboardNow)
            return;
        keyboardNow = k;
        const p = prefsStore.get();
        if (p)
            showLanguages(p);
    })
        .catch(() => {
        // Not available (older backend): "unknown".
    });
}
/** The picker: Keyboard / Detect / English / Hebrew, or "Other…" with the code field. */
function languagePick(codes, keyboard) {
    if (keyboard)
        return "keyboard";
    if (codes.length === 0)
        return "";
    if (hebrewChosen(codes))
        return "he";
    if (codes.length === 1 && codes[0].trim().toLowerCase().split(/[-_]/)[0] === "en")
        return "en";
    return "other";
}
function showLanguagePick(codes, keyboard) {
    const sel = $("language_pick");
    const box = $("languages_other");
    // Keep "Other…" while its field is open.
    if (box.classList.contains("is-editing"))
        return;
    const v = languagePick(codes, keyboard);
    sel.value = v;
    box.hidden = v !== "other";
}
function pickLanguage() {
    const v = $("language_pick").value;
    const box = $("languages_other");
    const input = $("languages");
    if (v === "other") {
        box.hidden = false;
        box.querySelector(".lw-edit-btn")?.click();
        return;
    }
    box.hidden = true;
    // Keyboard language (D37): no fixed languages behind it, so an unknown
    // layout is detected automatically.
    const codes = v === "keyboard" ? "" : v;
    input.value = codes;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    const p = prefsStore.get();
    if (p)
        showLanguages(p, codes, v === "keyboard");
    if (v === "keyboard")
        refreshKeyboard();
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
            meeting: $("meeting").value.trim(),
        },
    };
    next.model_path = $("model_path").value.trim() || null;
    next.asr_engine = $("asr_engine").value;
    next.parakeet_model_dir = $("parakeet_model_dir").value.trim() || null;
    next.hebrew_model = $("hebrew_model").value;
    // Choosing another Hebrew model leaves a server one (D41) behind.
    if (next.hebrew_model !== prefsStore.get()?.hebrew_model)
        next.hebrew_model_dir = null;
    next.model_source = $("model_source").value;
    next.microphone = $("microphone").value || null;
    for (const f of BOOL_FIELDS)
        next[f] = $(f).checked;
    next.paste_method = $("paste_method").value;
    next.keyboard_language = $("language_pick").value === "keyboard";
    next.languages = $("languages")
        .value.split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    next.max_dictation_minutes = Math.max(1, parseInt($("max_dictation_minutes").value, 10) || 20);
    next.theme = $("theme").value;
    // Hands-free auto-stop (D19): 0 = off, else 0.3-10 s of silence.
    const hf = parseFloat($("hands_free_silence_s").value);
    next.hands_free_silence_ms =
        !Number.isFinite(hf) || hf <= 0 ? 0 : Math.round(Math.min(10, Math.max(0.3, hf)) * 1000);
    return next;
}
export const general = {
    init() {
        $("asr_engine").addEventListener("change", showEngineFields);
        $("language_pick").addEventListener("change", pickLanguage);
        // D37: the layout may have changed while Settings was in the background.
        window.addEventListener("focus", () => {
            if (prefsStore.get()?.keyboard_language)
                refreshKeyboard();
        });
        // The Hebrew hint depends on what is downloaded and the Hebrew choice.
        models.onChange(() => {
            const p = prefsStore.get();
            if (p)
                showLanguages(p);
        });
        for (const e of document.querySelectorAll(".lw-edit"))
            wireEdit(e);
        // Theme applies at once; the save follows through data-save="prefs".
        $("theme").addEventListener("change", (e) => setTheme(e.target.value));
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
