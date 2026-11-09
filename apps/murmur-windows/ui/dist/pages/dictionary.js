// Dictionary and Snippets: two views of the one [{word, replacement,
// isSnippet}] list in prefs. Dictionary = vocabulary hints (names, jargon)
// and always-rewrite replacements ("heard → word"), plus learn-from-edits
// suggestions (added only on an explicit click) and the JSON import/export
// shared with Android. Snippets = say a trigger, get the text pasted.
//
// Layout follows docs/design/app-design/PageDictionary.dc.html and
// PageSnippets.dc.html. Every add, edit, delete and import is saved at
// once (with an Undo), so there is no separate Save button. The design's
// "Learned ✨" marker and tab are left out: entries carry no such flag.
import { $, button, el, setStatus } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
// Page-local icons (24x24 stroke paths, same style as lib/icons.ts).
const SORT_ICON = "M8 4v16M4 8l4-4 4 4M16 20V4M12 16l4 4 4-4";
const EDIT_ICON = "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4";
const DICT_TEMPLATE = `
<div class="lw-page lw-dict">
  <div class="lw-page-head">
    <h1 class="fl-page-title">Dictionary</h1>
    <label class="fl-btn fl-btn-secondary lw-file" title="Add the words and snippets from a JSON file">Import<input id="dict_file" type="file" accept=".json,application/json" /></label>
    <button type="button" class="fl-btn fl-btn-primary" id="dict_add">${iconSvg(ICON.plus, 16)}Add new</button>
  </div>

  <div class="fl-card lw-learn">
    <div class="lw-learn-head">
      <div class="lw-grow">
        <div class="fl-row-title">Learn from my corrections</div>
        <div class="fl-row-desc">When you fix a word right after dictating, it's suggested here so it's heard right next time.</div>
      </div>
      <input type="checkbox" class="fl-toggle" id="dict_learn" aria-label="Learn from my corrections" />
    </div>
    <ul id="sugg_list" class="lw-plain lw-sugg" hidden></ul>
  </div>

  <div id="dict_form_slot"></div>

  <div class="fl-tabs">
    <div class="fl-tabs-list" role="tablist" id="dict_tabs">
      <button type="button" role="tab" class="fl-tab is-active" aria-selected="true" data-tab="all">All</button>
      <button type="button" role="tab" class="fl-tab" aria-selected="false" data-tab="words">Words</button>
      <button type="button" role="tab" class="fl-tab" aria-selected="false" data-tab="replacements">Replacements</button>
    </div>
    <div class="fl-tabs-actions">
      <input id="dict_filter" class="fl-input lw-dict-search" type="search" placeholder="Search" spellcheck="false" aria-label="Search the dictionary" hidden />
      <button type="button" class="fl-iconbtn is-toolbar" id="dict_search" title="Search" aria-label="Search">${iconSvg(ICON.search, 20)}</button>
      <button type="button" class="fl-iconbtn is-toolbar" id="dict_sort" title="Sort A to Z" aria-label="Sort A to Z" aria-pressed="false">${iconSvg(SORT_ICON, 20)}</button>
    </div>
  </div>

  <div>
    <div class="fl-list" id="dict_rows"></div>
    <div class="fl-empty" id="dict_empty" hidden>
      <div class="fl-empty-title" id="dict_empty_title">No words yet</div>
      <div class="fl-empty-body" id="dict_empty_body">Add names and jargon so they're heard and spelled your way.</div>
    </div>
    <div class="lw-dict-status"><span id="dict_status" class="lw-status"></span></div>
  </div>
  <p class="lw-dict-note">Words here help Parakeet and Whisper hear names correctly, and the AI cleanup spell them your way. With "heard as", what was heard is always rewritten. They're sent to the cloud only if you allow it in Settings.</p>

  <section class="lw-section">
    <h3 class="fl-group-title">Import and export</h3>
    <div class="fl-group">
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Export</div><div class="fl-row-desc">A JSON list of <code>{"word", "replacement", "isSnippet"}</code>, the same file the Android app uses. Words and snippets.</div></div>
        <div class="fl-row-control">
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dict_export">Save to Downloads</button>
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dict_copy">Copy JSON</button>
        </div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Replace all from a file</div><div class="fl-row-desc">The Import button adds to your list. This swaps every word and snippet for the file's.</div></div>
        <div class="fl-row-control">
          <label class="fl-btn fl-btn-secondary fl-btn-sm lw-file">Choose file…<input id="dict_file_replace" type="file" accept=".json,application/json" /></label>
        </div>
      </div>
    </div>
    <p class="lw-status-line" id="io_status"></p>
  </section>
</div>`;
const SNIP_TEMPLATE = `
<div class="lw-page lw-dict">
  <div class="lw-page-head">
    <div class="lw-grow lw-head-text">
      <h1 class="fl-page-title">Snippets</h1>
      <span class="lw-head-sub">Say a short phrase, get the full text pasted.</span>
    </div>
    <input id="snip_filter" class="fl-input lw-dict-search" type="search" placeholder="Search" spellcheck="false" aria-label="Search snippets" hidden />
    <button type="button" class="fl-iconbtn is-toolbar" id="snip_search" title="Search" aria-label="Search">${iconSvg(ICON.search, 20)}</button>
    <label class="fl-btn fl-btn-secondary lw-file" title="Add the words and snippets from a JSON file">Import<input id="snip_file" type="file" accept=".json,application/json" /></label>
    <button type="button" class="fl-btn fl-btn-primary" id="snip_add">${iconSvg(ICON.plus, 16)}Add new</button>
  </div>
  <div id="snip_form_slot"></div>
  <div>
    <div class="fl-list" id="snip_rows"></div>
    <div class="fl-empty" id="snip_empty" hidden>
      <div class="fl-empty-title" id="snip_empty_title">No snippets yet</div>
      <div class="fl-empty-body" id="snip_empty_body">Add one: say "my calendar link", get the link pasted.</div>
    </div>
    <div class="lw-dict-status"><span id="snip_status" class="lw-status"></span></div>
  </div>
</div>`;
let dict = [];
let tab = "all";
let sortAz = false;
/** Which entry each page is editing: an index into `dict`, "new", or none. */
const editing = { dict: null, snip: null };
const sameDict = (a, b) => JSON.stringify(a) === JSON.stringify(b);
function takeDict(p) {
    dict = p.dictionary.map((e) => ({ ...e }));
    render();
}
// ------------------------------------------------------------------ saving
let saveChain = Promise.resolve();
/** Save `next` as the dictionary, then show `msg` (with an Undo back to
 *  the list before this change) in `statusId`. Saves run one at a time. */
function persist(next, statusId, msg) {
    const before = dict.map((e) => ({ ...e }));
    const run = async () => {
        try {
            await prefsStore.save({ dictionary: next.map((e) => ({ ...e })) });
            // The backend normalises (trims, drops blanks, collapses duplicates).
            takeDict(await prefsStore.load());
            setStatus(statusId, msg, true);
            const undo = button("Undo", () => void persist(before, statusId, "Undone"), "fl-btn fl-btn-link lw-undo");
            if (msg !== "Undone")
                $(statusId).append(" ", undo);
            void loadSuggestions();
            return true;
        }
        catch (e) {
            setStatus(statusId, String(e), false);
            return false;
        }
    };
    const p = saveChain.then(run, run);
    saveChain = p;
    return p;
}
// ------------------------------------------------------------------ rows
/** Dictionary entries as the design shows them: what was heard, and the
 *  word written. A plain word has nothing heard. */
function heardWord(e) {
    return e.replacement ? { heard: e.word, word: e.replacement } : { heard: "", word: e.word };
}
function rowActions(idx, snippet, label) {
    const box = el("span", "lw-row-actions");
    const edit = el("button", "fl-iconbtn is-toolbar");
    edit.type = "button";
    edit.title = "Edit";
    edit.setAttribute("aria-label", `Edit ${label}`);
    edit.append(iconEl(EDIT_ICON, 16));
    edit.addEventListener("click", (ev) => {
        ev.stopPropagation();
        openForm(snippet, idx);
    });
    const del = el("button", "fl-iconbtn is-toolbar");
    del.type = "button";
    del.title = "Remove";
    del.setAttribute("aria-label", `Remove ${label}`);
    del.append(iconEl(ICON.trash, 16));
    del.addEventListener("click", (ev) => {
        ev.stopPropagation();
        const next = dict.filter((_, i) => i !== idx);
        void persist(next, snippet ? "snip_status" : "dict_status", `Removed "${label}"`);
    });
    box.append(edit, del);
    return box;
}
function clickable(row, snippet, idx) {
    row.tabIndex = 0;
    row.addEventListener("click", () => openForm(snippet, idx));
    row.addEventListener("keydown", (ev) => {
        if (ev.key === "Enter" && ev.target === row)
            openForm(snippet, idx);
    });
}
function wordRow(e, idx) {
    const row = el("div", "fl-list-row lw-dict-row");
    const { heard, word } = heardWord(e);
    const text = el("span", "lw-dict-text");
    if (heard)
        text.append(el("span", "lw-heard", heard), el("span", "lw-arrow", "→"));
    text.append(el("span", "lw-word", word));
    text.lastElementChild?.setAttribute("title", word);
    row.append(text, rowActions(idx, false, word));
    clickable(row, false, idx);
    return row;
}
function snippetRow(e, idx) {
    const row = el("div", "fl-list-row lw-dict-row lw-snip-row");
    const say = el("span", "lw-snip-say", e.word);
    const arrow = el("span", "lw-arrow", "→");
    arrow.setAttribute("aria-hidden", "true");
    const paste = el("span", "lw-snip-paste", (e.replacement ?? "").replace(/\s*\n\s*/g, " ⏎ "));
    paste.title = e.replacement ?? "";
    row.append(say, arrow, paste, rowActions(idx, true, e.word));
    clickable(row, true, idx);
    return row;
}
// ------------------------------------------------------------------ add / edit form
function field(label, control) {
    const l = el("label", "lw-field");
    l.append(el("span", "fl-overline", label), control);
    return l;
}
function input(value, placeholder) {
    const i = el("input", "fl-input");
    i.type = "text";
    i.value = value;
    i.placeholder = placeholder;
    i.spellcheck = false;
    return i;
}
/** The add / edit card (PageSnippets.dc.html's inline form, reused for
 *  dictionary words). `idx` is the entry being edited, or "new". */
function entryForm(snippet, idx) {
    const e = idx === "new" ? null : dict[idx];
    const form = el("form", "lw-dict-form");
    form.noValidate = true;
    const grid = el("div", "lw-dict-form-grid");
    const err = el("span", "lw-status err lw-grow");
    let read;
    if (snippet) {
        const say = input(e?.word ?? "", "my calendar link");
        const paste = el("textarea", "fl-input lw-textarea");
        paste.rows = 1;
        paste.value = e?.replacement ?? "";
        paste.placeholder = "https://cal.com/you/30min";
        const fit = () => {
            paste.style.height = "auto";
            paste.style.height = `${Math.min(paste.scrollHeight, 200)}px`;
        };
        paste.addEventListener("input", fit);
        requestAnimationFrame(fit);
        grid.append(field("When I say", say), field("Paste", paste));
        read = () => {
            const w = say.value.trim();
            const r = paste.value.trim();
            if (!w)
                return "Say what? Fill in the phrase.";
            if (!r)
                return "Fill in the text to paste.";
            return { word: w, replacement: r, isSnippet: true };
        };
    }
    else {
        const hw = e ? heardWord(e) : { heard: "", word: "" };
        const word = input(hw.word, "Hetzner");
        const heard = input(hw.heard, "Optional, e.g. het snore");
        grid.append(field("Word", word), field("Heard as", heard));
        read = () => {
            const w = word.value.trim();
            const h = heard.value.trim();
            if (!w)
                return "Fill in the word.";
            return h ? { word: h, replacement: w, isSnippet: false } : { word: w, replacement: null, isSnippet: false };
        };
    }
    const foot = el("div", "lw-dict-form-foot");
    const cancel = button("Cancel", () => closeForm(snippet), "fl-btn fl-btn-ghost fl-btn-sm");
    const save = el("button", "fl-btn fl-btn-primary fl-btn-sm", "Save");
    save.type = "submit";
    foot.append(err, cancel, save);
    form.append(grid, foot);
    form.addEventListener("submit", (ev) => {
        ev.preventDefault();
        const entry = read();
        if (typeof entry === "string") {
            err.textContent = entry;
            return;
        }
        const next = dict.map((x) => ({ ...x }));
        if (idx === "new")
            next.unshift(entry);
        else
            next[idx] = entry;
        const label = entry.isSnippet || !entry.replacement ? entry.word : entry.replacement;
        save.disabled = true;
        void persist(next, snippet ? "snip_status" : "dict_status", idx === "new" ? `Added "${label}"` : `Saved "${label}"`).then((ok) => {
            save.disabled = false;
            if (ok)
                closeForm(snippet);
        });
    });
    form.addEventListener("keydown", (ev) => {
        if (ev.key === "Escape") {
            ev.preventDefault();
            closeForm(snippet);
        }
        else if (ev.key === "Enter" && ev.ctrlKey) {
            ev.preventDefault();
            form.requestSubmit();
        }
    });
    return form;
}
function openForm(snippet, idx) {
    editing[snippet ? "snip" : "dict"] = idx;
    if (idx === "new")
        $(snippet ? "snip_filter" : "dict_filter").value = "";
    render();
    const host = idx === "new" ? $(snippet ? "snip_form_slot" : "dict_form_slot") : $(snippet ? "snip_rows" : "dict_rows");
    host.querySelector(".lw-dict-form input")?.focus();
}
function closeForm(snippet) {
    editing[snippet ? "snip" : "dict"] = null;
    render();
}
// ------------------------------------------------------------------ render
function matches(e, f) {
    return !f || e.word.toLowerCase().includes(f) || (e.replacement ?? "").toLowerCase().includes(f);
}
function inTab(e) {
    return tab === "all" || (tab === "replacements") === !!e.replacement;
}
function sorted(items, key) {
    return sortAz ? [...items].sort((a, b) => key(a[0]).localeCompare(key(b[0]), undefined, { sensitivity: "base" })) : items;
}
function showEmpty(prefix, total, shown, filter, none) {
    const empty = $(`${prefix}_empty`);
    empty.hidden = shown > 0;
    if (shown > 0)
        return;
    const [title, body] = total === 0
        ? none
        : filter
            ? [`Nothing matches "${filter}"`, "Try another word, or clear the search."]
            : ["Nothing here yet", tab === "replacements" ? 'Add a word with "heard as" to always rewrite it.' : "Every entry here rewrites what was heard."];
    $(`${prefix}_empty_title`).textContent = title;
    $(`${prefix}_empty_body`).textContent = body;
}
function renderList(snippet) {
    const prefix = snippet ? "snip" : "dict";
    const f = $(`${prefix}_filter`).value.trim().toLowerCase();
    const mine = [];
    dict.forEach((e, i) => {
        if (!!e.isSnippet === snippet)
            mine.push([e, i]);
    });
    const shown = sorted(mine.filter(([e]) => matches(e, f) && (snippet || inTab(e))), snippet ? (e) => e.word : (e) => heardWord(e).word);
    const edit = editing[prefix];
    const rows = shown.map(([e, i]) => {
        if (edit === i) {
            const r = el("div", "fl-list-row lw-dict-editrow");
            r.append(entryForm(snippet, i));
            return r;
        }
        return snippet ? snippetRow(e, i) : wordRow(e, i);
    });
    const list = $(`${prefix}_rows`);
    list.replaceChildren(...rows);
    list.hidden = rows.length === 0;
    $(`${prefix}_form_slot`).replaceChildren(...(edit === "new" ? [entryForm(snippet, "new")] : []));
    showEmpty(prefix, mine.length, rows.length, f, snippet
        ? ["No snippets yet", 'Add one: say "my calendar link", get the link pasted.']
        : ["No words yet", "Add names and jargon so they're heard and spelled your way."]);
}
function render() {
    // An index being edited may have moved or gone after a reload.
    for (const k of ["dict", "snip"]) {
        const i = editing[k];
        if (typeof i === "number" && (!dict[i] || !!dict[i].isSnippet !== (k === "snip")))
            editing[k] = null;
    }
    renderList(false);
    renderList(true);
}
// ------------------------------------------------------------------ search, tabs, sort
function wireSearch(prefix) {
    const box = $(`${prefix}_filter`);
    const btn = $(`${prefix}_search`);
    box.addEventListener("input", render);
    box.addEventListener("keydown", (ev) => {
        if (ev.key === "Escape") {
            box.value = "";
            box.hidden = true;
            btn.hidden = false;
            render();
            btn.focus();
        }
    });
    box.addEventListener("blur", () => {
        if (!box.value) {
            box.hidden = true;
            btn.hidden = false;
        }
    });
    btn.addEventListener("click", () => {
        box.hidden = false;
        btn.hidden = true;
        box.focus();
    });
}
function wireTabs() {
    const list = $("dict_tabs");
    list.addEventListener("click", (ev) => {
        const b = ev.target.closest("[data-tab]");
        if (!b)
            return;
        tab = b.dataset.tab;
        for (const t of list.querySelectorAll("[data-tab]")) {
            const on = t === b;
            t.classList.toggle("is-active", on);
            t.setAttribute("aria-selected", String(on));
        }
        render();
    });
    const sort = $("dict_sort");
    sort.addEventListener("click", () => {
        sortAz = !sortAz;
        sort.setAttribute("aria-pressed", String(sortAz));
        sort.classList.toggle("is-active", sortAz);
        sort.title = sortAz ? "Newest first" : "Sort A to Z";
        render();
    });
}
// ------------------------------------------------------------------ import / export
async function importFile(input, replace, statusId) {
    const file = input.files?.[0];
    if (!file)
        return;
    try {
        const json = await file.text();
        const r = await invoke("dictionary_import", { entries: dict, json, replace });
        const skipped = r.skipped ? `, ${r.skipped} invalid skipped` : "";
        const what = replace ? `Replaced with ${r.entries.length} entries` : `${r.added} added, ${r.updated} updated`;
        await persist(r.entries, statusId, `${what}${skipped}.`);
    }
    catch (e) {
        setStatus(statusId, String(e), false);
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
    const ul = $("sugg_list");
    ul.hidden = list.length === 0;
    ul.replaceChildren(...list.map((s) => {
        const li = el("li", "lw-sugg-row");
        const what = el("span", "lw-dict-text");
        what.append(el("span", "lw-heard", s.heard), el("span", "lw-arrow", "→"), el("span", "lw-word", s.word));
        const meta = el("span", "lw-muted", `${s.app.replace(/\.exe$/i, "")}${s.count > 1 ? ` ×${s.count}` : ""}`);
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
// ------------------------------------------------------------------ pages
export const dictionaryPage = {
    id: "dictionary",
    area: "main",
    template: DICT_TEMPLATE,
    init() {
        wireSearch("dict");
        wireTabs();
        $("dict_add").addEventListener("click", () => openForm(false, "new"));
        const file = $("dict_file");
        file.addEventListener("change", () => void importFile(file, false, "dict_status"));
        const rep = $("dict_file_replace");
        rep.addEventListener("change", () => void importFile(rep, true, "io_status"));
        $("dict_export").addEventListener("click", () => void exportFile());
        $("dict_copy").addEventListener("click", () => void copyJson());
        const learn = $("dict_learn");
        learn.addEventListener("change", () => {
            prefsStore.save({ learn_from_edits: learn.checked }).catch((e) => {
                learn.checked = !learn.checked;
                setStatus("dict_status", String(e), false);
            });
        });
        void onEvent("suggestions:changed", () => void loadSuggestions());
        // Every add / edit / delete is saved at once, so the saved list is
        // the page's list: take changes made elsewhere (e.g. an import on the
        // other page, or a suggestion accepted) whenever they differ.
        let loaded = false;
        prefsStore.subscribe((p) => {
            learn.checked = !!p.learn_from_edits;
            if (loaded && sameDict(p.dictionary, dict))
                return;
            takeDict(p);
            if (!loaded)
                void loadSuggestions();
            loaded = true;
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
        wireSearch("snip");
        $("snip_add").addEventListener("click", () => openForm(true, "new"));
        const file = $("snip_file");
        file.addEventListener("change", () => void importFile(file, false, "snip_status"));
    },
};
