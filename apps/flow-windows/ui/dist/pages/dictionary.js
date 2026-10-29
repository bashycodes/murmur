// Dictionary: edits the [{word, replacement, isSnippet}] list, shows
// learn-from-edits suggestions (added only on an explicit click) and
// imports/exports the JSON shared with Android.
import { $, button, el, html, setStatus } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
const TEMPLATE = `
<section class="panel" data-panel="dictionary">
  <fieldset id="sugg_box" hidden>
    <legend>Suggestions from your corrections</legend>
    <p class="hint">You changed these words after a dictation. Add them so they are recognised next time.</p>
    <ul id="sugg_list" class="sugg"></ul>
  </fieldset>

  <fieldset>
    <legend>Dictionary &amp; snippets</legend>
    <div class="toolbar">
      <input id="dict_filter" type="search" placeholder="Filter" spellcheck="false" />
      <button type="button" id="dict_add">Add</button>
    </div>
    <table class="dict">
      <thead><tr><th>Word or trigger</th><th>Replace with / snippet text</th><th title="Snippet">Snip</th><th></th></tr></thead>
      <tbody id="dict_rows"></tbody>
    </table>
    <p class="hint">Word only = vocabulary hint (names, jargon). With a replacement = always rewritten. Snippet = saying the trigger inserts the text.</p>
    <div class="toolbar">
      <button type="button" id="dict_save">Save dictionary</button>
      <span id="dict_status"></span>
    </div>
  </fieldset>

  <fieldset>
    <legend>Import / export</legend>
    <p class="hint">JSON list of <code>{"word", "replacement", "isSnippet"}</code>, the same file the Android app uses.</p>
    <div class="toolbar">
      <label class="row">Import <input id="dict_file" type="file" accept=".json,application/json" /></label>
      <select id="dict_mode">
        <option value="merge">Merge</option>
        <option value="replace">Replace all</option>
      </select>
    </div>
    <div class="toolbar">
      <button type="button" id="dict_export">Export to Downloads</button>
      <button type="button" id="dict_copy" class="secondary">Copy JSON</button>
    </div>
    <p id="io_status" class="hint"></p>
  </fieldset>
</section>`;
let dict = [];
function takeDict(p) {
    dict = p.dictionary.map((e) => ({ ...e }));
    renderDict();
}
function dictRow(e, idx) {
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
function renderDict() {
    const f = $("dict_filter").value.trim().toLowerCase();
    const rows = [];
    dict.forEach((e, i) => {
        if (!f || e.word.toLowerCase().includes(f) || (e.replacement ?? "").toLowerCase().includes(f))
            rows.push(dictRow(e, i));
    });
    $("dict_rows").replaceChildren(...rows);
}
async function saveDict() {
    try {
        await prefsStore.save({ dictionary: dict.map((e) => ({ ...e })) });
        // The backend normalises (trims, drops blanks, collapses duplicates).
        takeDict(await prefsStore.load());
        setStatus("dict_status", "Saved", true);
        await loadSuggestions();
    }
    catch (e) {
        setStatus("dict_status", String(e), false);
    }
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
        setStatus("io_status", `${r.added} added, ${r.updated} updated${skipped}. Press "Save dictionary" to keep them.`, true);
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
        const what = el("span", "what");
        what.append(el("q", undefined, s.heard), " → ", el("strong", undefined, s.word));
        const meta = el("span", "hint", ` ${s.app.replace(/\.exe$/i, "")}${s.count > 1 ? ` ×${s.count}` : ""}`);
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
        li.append(what, meta, button("Add word", () => void accept(false)), button("Add as replacement", () => void accept(true), "small secondary"), button("Dismiss", () => void invoke("suggestion_dismiss", { word: s.word }).then(loadSuggestions), "small secondary"));
        return li;
    }));
}
export const dictionaryPage = {
    id: "dictionary",
    mount(root) {
        root.append(html(TEMPLATE));
        $("dict_filter").addEventListener("input", renderDict);
        $("dict_add").addEventListener("click", () => {
            $("dict_filter").value = "";
            dict.unshift({ word: "", replacement: null, isSnippet: false });
            renderDict();
            $("dict_rows").querySelector("input")?.focus();
        });
        $("dict_save").addEventListener("click", () => void saveDict());
        $("dict_file").addEventListener("change", () => void importFile());
        $("dict_export").addEventListener("click", () => void exportFile());
        $("dict_copy").addEventListener("click", () => void copyJson());
        void onEvent("suggestions:changed", () => void loadSuggestions());
    },
    async load() {
        const p = prefsStore.get();
        if (p)
            takeDict(p);
        await loadSuggestions();
    },
    show() {
        void loadSuggestions();
    },
};
