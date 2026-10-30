// Dictionary and Snippets: two views of the one [{word, replacement,
// isSnippet}] list in prefs. Dictionary = vocabulary hints (names, jargon)
// and always-rewrite replacements, plus learn-from-edits suggestions (added
// only on an explicit click) and the JSON import/export shared with
// Android. Snippets = say a trigger, get the text pasted.
import { $, button, el, setStatus } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
const DICT_TEMPLATE = `
<div class="lw-page">
  <div class="lw-page-head">
    <h1 class="fl-page-title">Dictionary</h1>
    <input id="dict_filter" class="fl-input" type="search" placeholder="Search" spellcheck="false" aria-label="Search the dictionary" />
    <button type="button" class="fl-btn fl-btn-primary" id="dict_add">${iconSvg(ICON.plus, 16)}Add new</button>
  </div>

  <div class="fl-callout fl-callout-lilac" id="sugg_box" hidden>
    <div class="fl-grow">
      <div class="fl-callout-title">Suggestions from your corrections</div>
      <div class="fl-callout-body">You changed these words after a dictation. Add them so they are recognised next time.</div>
      <ul id="sugg_list" class="lw-plain lw-sugg"></ul>
    </div>
  </div>

  <div class="fl-list" id="dict_rows"></div>
  <div class="fl-empty" id="dict_empty" hidden><div class="fl-empty-title">No words yet</div><div class="fl-empty-body">Add names and jargon so they are heard and spelled your way.</div></div>
  <div class="lw-actions">
    <button type="button" class="fl-btn fl-btn-primary" id="dict_save">Save dictionary</button>
    <span id="dict_status" class="lw-status"></span>
  </div>
  <p class="fl-help">A word alone helps Parakeet and Whisper hear it and the AI cleanup spell it your way. With "always write as", what you say is always rewritten. Words are sent to OpenRouter only if you allow it in Settings.</p>

  <section class="lw-section">
    <h3 class="fl-group-title">Import and export</h3>
    <div class="fl-group">
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Import</div><div class="fl-row-desc">JSON list of <code>{"word", "replacement", "isSnippet"}</code>, the same file the Android app uses. Words and snippets.</div></div>
        <div class="fl-row-control">
          <select id="dict_mode" class="fl-select" aria-label="Import mode">
            <option value="merge">Merge</option>
            <option value="replace">Replace all</option>
          </select>
          <label class="fl-btn fl-btn-secondary fl-btn-sm lw-file">Choose file…<input id="dict_file" type="file" accept=".json,application/json" /></label>
        </div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Export</div><div class="fl-row-desc" id="io_status"></div></div>
        <div class="fl-row-control">
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dict_export">Save to Downloads</button>
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dict_copy">Copy JSON</button>
        </div>
      </div>
    </div>
  </section>
</div>`;
const SNIP_TEMPLATE = `
<div class="lw-page">
  <div class="lw-page-head">
    <div class="lw-grow">
      <h1 class="fl-page-title">Snippets</h1>
      <p class="lw-sub">Say a short phrase, get the full text pasted.</p>
    </div>
    <input id="snip_filter" class="fl-input" type="search" placeholder="Search" spellcheck="false" aria-label="Search snippets" />
    <button type="button" class="fl-btn fl-btn-primary" id="snip_add">${iconSvg(ICON.plus, 16)}Add new</button>
  </div>
  <div class="fl-list" id="snip_rows"></div>
  <div class="fl-empty" id="snip_empty" hidden><div class="fl-empty-title">No snippets yet</div><div class="fl-empty-body">Add one: say "my calendar link", get the link pasted.</div></div>
  <div class="lw-actions">
    <button type="button" class="fl-btn fl-btn-primary" id="snip_save">Save snippets</button>
    <span id="snip_status" class="lw-status"></span>
  </div>
</div>`;
let dict = [];
function takeDict(p) {
    dict = p.dictionary.map((e) => ({ ...e }));
    renderDict();
}
function deleteButton(idx, label) {
    const del = el("button", "fl-iconbtn is-toolbar");
    del.type = "button";
    del.title = "Remove";
    del.setAttribute("aria-label", `Remove ${label}`);
    del.append(iconEl(ICON.trash, 16));
    del.addEventListener("click", () => {
        dict.splice(idx, 1);
        renderDict();
    });
    return del;
}
function wordRow(e, idx) {
    const row = el("div", "fl-list-row lw-dict-row");
    const word = el("input", "lw-cell lw-cell-word");
    word.value = e.word;
    word.spellcheck = false;
    word.placeholder = "Word or name";
    word.setAttribute("aria-label", "Word");
    word.addEventListener("input", () => (dict[idx].word = word.value));
    const rep = el("input", "lw-cell");
    rep.value = e.replacement ?? "";
    rep.placeholder = "Always write as… (optional)";
    rep.setAttribute("aria-label", "Always write as");
    rep.addEventListener("input", () => (dict[idx].replacement = rep.value || null));
    row.append(word, el("span", "lw-arrow", "→"), rep, deleteButton(idx, e.word));
    return row;
}
function snippetRow(e, idx) {
    const row = el("div", "fl-list-row lw-dict-row");
    const say = el("input", "lw-cell lw-cell-word");
    say.value = e.word;
    say.spellcheck = false;
    say.placeholder = "When I say";
    say.setAttribute("aria-label", "When I say");
    say.addEventListener("input", () => (dict[idx].word = say.value));
    const text = el("textarea", "lw-cell");
    text.rows = 1;
    text.value = e.replacement ?? "";
    text.placeholder = "Paste this";
    text.setAttribute("aria-label", "Paste");
    text.addEventListener("input", () => (dict[idx].replacement = text.value || null));
    row.append(say, el("span", "lw-arrow", "→"), text, deleteButton(idx, e.word));
    return row;
}
function matches(e, f) {
    return !f || e.word.toLowerCase().includes(f) || (e.replacement ?? "").toLowerCase().includes(f);
}
function renderDict() {
    const fw = $("dict_filter").value.trim().toLowerCase();
    const fs = $("snip_filter").value.trim().toLowerCase();
    const words = [];
    const snips = [];
    dict.forEach((e, i) => {
        if (e.isSnippet) {
            if (matches(e, fs))
                snips.push(snippetRow(e, i));
        }
        else if (matches(e, fw))
            words.push(wordRow(e, i));
    });
    $("dict_rows").replaceChildren(...words);
    $("snip_rows").replaceChildren(...snips);
    $("dict_rows").hidden = words.length === 0;
    $("snip_rows").hidden = snips.length === 0;
    $("dict_empty").hidden = words.length > 0;
    $("snip_empty").hidden = snips.length > 0;
}
async function saveDict(statusId) {
    try {
        await prefsStore.save({ dictionary: dict.map((e) => ({ ...e })) });
        // The backend normalises (trims, drops blanks, collapses duplicates).
        takeDict(await prefsStore.load());
        setStatus(statusId, "Saved", true);
        await loadSuggestions();
    }
    catch (e) {
        setStatus(statusId, String(e), false);
    }
}
function addEntry(snippet) {
    $(snippet ? "snip_filter" : "dict_filter").value = "";
    dict.unshift({ word: "", replacement: null, isSnippet: snippet });
    renderDict();
    $(snippet ? "snip_rows" : "dict_rows").querySelector("input")?.focus();
}
async function importFile() {
    const input = $("dict_file");
    const file = input.files?.[0];
    if (!file)
        return;
    try {
        const json = await file.text();
        const r = await invoke("dictionary_import", {
            entries: dict,
            json,
            replace: $("dict_mode").value === "replace",
        });
        dict = r.entries;
        renderDict();
        const skipped = r.skipped ? `, ${r.skipped} invalid skipped` : "";
        setStatus("io_status", `${r.added} added, ${r.updated} updated${skipped}. Press Save to keep them.`, true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
    finally {
        input.value = "";
    }
}
async function exportFile() {
    try {
        const path = await invoke("dictionary_export", { entries: dict });
        setStatus("io_status", `Saved ${dict.length} entries to ${path}`, true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
}
async function copyJson() {
    try {
        const text = await invoke("dictionary_export_text", { entries: dict });
        await invoke("clipboard_copy", { text });
        setStatus("io_status", "JSON copied to the clipboard", true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
}
// ------------------------------------------------------------------ suggestions
function showSuggestionCount(n) {
    const b = $("sugg_badge");
    b.hidden = n === 0;
    b.textContent = String(n);
}
async function loadSuggestions() {
    const list = await invoke("suggestions_list");
    showSuggestionCount(list.length);
    $("sugg_box").hidden = list.length === 0;
    $("sugg_list").replaceChildren(...list.map((s) => {
        const li = el("li");
        const what = el("span", "lw-grow");
        what.append(el("q", undefined, s.heard), " → ", el("strong", undefined, s.word));
        const meta = el("span", "lw-muted", ` ${s.app.replace(/\.exe$/i, "")}${s.count > 1 ? ` ×${s.count}` : ""}`);
        const accept = async (asReplacement) => {
            try {
                const p = await invoke("suggestion_accept", { word: s.word, asReplacement });
                prefsStore.adopt(p);
                takeDict(p);
                setStatus("dict_status", `Added "${s.word}"`, true);
            }
            catch (e) {
                setStatus("dict_status", String(e), false);
            }
            await loadSuggestions();
        };
        li.append(what, meta, button("Add word", () => void accept(false)), button("Add as replacement", () => void accept(true), "fl-btn fl-btn-ghost fl-btn-sm"), button("Dismiss", () => void invoke("suggestion_dismiss", { word: s.word }).then(loadSuggestions), "fl-btn fl-btn-ghost fl-btn-sm"));
        return li;
    }));
}
export const dictionaryPage = {
    id: "dictionary",
    area: "main",
    template: DICT_TEMPLATE,
    init() {
        $("dict_filter").addEventListener("input", renderDict);
        $("dict_add").addEventListener("click", () => addEntry(false));
        $("dict_save").addEventListener("click", () => void saveDict("dict_status"));
        $("dict_file").addEventListener("change", () => void importFile());
        $("dict_export").addEventListener("click", () => void exportFile());
        $("dict_copy").addEventListener("click", () => void copyJson());
        void onEvent("suggestions:changed", () => void loadSuggestions());
        // First load only: later saves come back through saveDict / accept,
        // so unsaved edits are not overwritten by another page's save.
        let loaded = false;
        prefsStore.subscribe((p) => {
            if (loaded)
                return;
            loaded = true;
            takeDict(p);
            void loadSuggestions();
        });
    },
    show() {
        void loadSuggestions();
    },
};
export const snippetsPage = {
    id: "snippets",
    area: "main",
    template: SNIP_TEMPLATE,
    init() {
        $("snip_filter").addEventListener("input", renderDict);
        $("snip_add").addEventListener("click", () => addEntry(true));
        $("snip_save").addEventListener("click", () => void saveDict("snip_status"));
    },
};
