// One folder of model files on this PC → English, Hebrew and auto-detect:
// setup's "Use model files on this PC" (onboarding/index.ts) and Settings →
// Speech to text → "Import from folder…" share this checklist. The folder
// is scanned by the backend (`model_folder_scan`) and the ticked rows are
// copied and checked there (`model_folder_import`, src/model_import.rs);
// the rows' wording is lib/folder-checklist.ts. A missing row's Download
// is the usual confirm-then-download (features/models.ts).
import { $, html } from "../lib/dom.js";
import { invoke } from "../lib/ipc.js";
import { canUse, downloadTarget, importLine, mbText, notes, pickOf, ROW_LABEL, ROWS, rowView, scanLine, } from "../lib/folder-checklist.js";
import { prefsStore } from "../lib/prefs-store.js";
import { models, pickPath } from "./models.js";
/** `model:download` id of a folder import (model_import::FOLDER_JOB_ID). */
const JOB_ID = "folder-import";
const rowHtml = (k) => `
<li class="lw-mf-row" data-mf-row="${k}">
  <input type="checkbox" class="lw-mf-tick" data-mf-use checked aria-label="Use this ${ROW_LABEL[k].toLowerCase()}" />
  <span class="lw-mf-text">
    <span class="lw-mf-label">${ROW_LABEL[k]}</span>
    <span class="lw-mf-name" data-mf-name></span>
    <span class="lw-mf-detail" data-mf-detail></span>
  </span>
  <span class="fl-badge fl-badge-light" data-mf-badge></span>
  <span class="lw-mf-actions">
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-mf-dl hidden>Download</button>
    <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" data-mf-skip hidden>Skip</button>
  </span>
</li>`;
/** The checklist's markup (hidden until a folder is picked); `id` is its root's. */
export const folderChecklist = (id) => `
<div class="lw-mf" id="${id}" hidden>
  <div class="lw-mf-head">
    <span class="lw-mf-folder lw-mono" data-mf-folder></span>
    <button type="button" class="lw-link" data-mf-pick>Choose another folder…</button>
  </div>
  <ul class="lw-mf-rows" aria-label="Models in this folder">${ROWS.map(rowHtml).join("")}</ul>
  <p class="lw-mf-line" data-mf-line role="status"></p>
  <ul class="lw-mf-notes" data-mf-notes hidden></ul>
  <div class="lw-mf-progress" data-mf-progress hidden>
    <div class="lw-ob-bar" role="progressbar" aria-label="Copying the models" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" data-mf-bar><span></span></div>
    <span class="lw-ob-dl-line" data-mf-progress-line></span>
    <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" data-mf-cancel>Cancel</button>
  </div>
  <div class="lw-mf-foot">
    <button type="button" class="fl-btn fl-btn-primary" data-mf-go disabled>Use these</button>
  </div>
</div>`;
const BADGE = { ok: "fl-badge-success", warn: "fl-badge-warning", err: "fl-badge-destructive", muted: "fl-badge-light" };
/** One checklist on screen: picks the folder, shows the rows, imports. */
export class FolderChecklist {
    constructor(root) {
        this.root = root;
        this.scan = null;
        this.result = null;
        this.use = { english: true, hebrew: true, lid: true };
        this.skipped = { english: false, hebrew: false, lid: false };
        this.importing = false;
        this.line = { text: "" };
        this.listeners = [];
        root.addEventListener("click", (e) => {
            const t = e.target;
            const row = t.closest("[data-mf-row]")?.dataset.mfRow;
            if (t.closest("[data-mf-pick]"))
                void this.choose();
            else if (t.closest("[data-mf-go]"))
                void this.go();
            else if (t.closest("[data-mf-cancel]"))
                void models.cancel(JOB_ID);
            else if (row && t.closest("[data-mf-skip]")) {
                this.skipped[row] = true;
                this.render();
            }
            else if (row && t.closest("[data-mf-dl]")) {
                const d = downloadTarget(row, models.catalog());
                if (d)
                    void models.confirmAndStart(d.id);
            }
        });
        root.addEventListener("change", (e) => {
            const t = e.target;
            const row = t.closest("[data-mf-row]")?.dataset.mfRow;
            if (row && t.matches("[data-mf-use]")) {
                this.use[row] = t.checked;
                this.render();
            }
        });
        models.onChange(() => this.render());
    }
    /** Called whenever what the checklist shows changes. */
    onChange(l) {
        this.listeners.push(l);
    }
    /** A folder is picked (the checklist is on screen). */
    get open() {
        return !this.root.hidden;
    }
    /** The import is running (Continue / Close wait for it). */
    get busy() {
        return this.importing;
    }
    /** "Use these" finished with at least one model set up. */
    get imported() {
        return !!this.result && ROWS.some((k) => this.result?.[k].state === "imported");
    }
    /** Back to nothing picked (hidden). */
    reset() {
        if (this.importing)
            return;
        this.scan = null;
        this.result = null;
        this.use = { english: true, hebrew: true, lid: true };
        this.skipped = { english: false, hebrew: false, lid: false };
        this.line = { text: "" };
        this.root.hidden = true;
        this.changed();
    }
    /** The folder picker, then the scan. False when nothing was picked. */
    async choose() {
        if (this.importing)
            return false;
        let path = null;
        try {
            path = await pickPath("folder", "Choose the folder with your model files");
        }
        catch (e) {
            console.error("pick_path", e);
        }
        if (!path)
            return false;
        await this.load(path);
        return true;
    }
    /** Scan `path` and show its rows. */
    async load(path) {
        this.reset();
        this.root.hidden = false;
        $folder(this.root).textContent = path;
        this.line = { text: "Looking through the folder…" };
        this.render();
        try {
            this.scan = await invoke("model_folder_scan", { path });
            this.line = { text: scanLine(this.scan) };
        }
        catch (e) {
            this.line = { text: `Couldn't read that folder: ${String(e)}`, ok: false };
        }
        this.render();
    }
    /** "Use these": import the ticked rows. */
    async go() {
        const scan = this.scan;
        if (!scan || !canUse(scan, this.use) || this.importing || models.busy())
            return;
        this.importing = true;
        this.line = { text: "" };
        this.render();
        try {
            this.result = await invoke("model_folder_import", { path: scan.folder, pick: pickOf(scan, this.use) });
            this.line = importLine(this.result);
        }
        catch (e) {
            this.line = { text: `Import failed: ${String(e)}`, ok: false };
        }
        finally {
            this.importing = false;
            await Promise.all([models.refresh().catch(() => []), prefsStore.load().catch(() => { })]);
            this.render();
        }
    }
    /** The model behind a row is usable on this PC already. */
    have(k) {
        const cat = models.catalog().filter((c) => c.engine !== "llm");
        if (k === "lid")
            return models.lidPresent() === true;
        if (k === "hebrew")
            return cat.some((c) => !!c.language && c.installed);
        const d = downloadTarget("english", cat);
        return !!d && !!models.entry(d.id)?.installed;
    }
    render() {
        const scan = this.scan;
        const rows = this.root.querySelector(".lw-mf-rows");
        rows.hidden = !scan;
        for (const k of ROWS) {
            if (!scan)
                break;
            const r = this.root.querySelector(`[data-mf-row="${k}"]`);
            const d = downloadTarget(k, models.catalog());
            const p = d ? models.progress(d.id) : undefined;
            const running = !!d && models.busy(d.id);
            const v = rowView({
                slot: scan[k],
                result: this.result?.[k],
                use: this.use[k],
                skipped: this.skipped[k],
                have: this.have(k),
                download: d,
                canStart: !!d && models.canStart(models.entry(d.id)),
                pct: running ? (p?.total ? Math.round((p.received / p.total) * 100) : 0) : null,
            });
            r.querySelector("[data-mf-name]").textContent = v.name;
            r.querySelector("[data-mf-detail]").textContent = v.detail;
            const badge = r.querySelector("[data-mf-badge]");
            badge.textContent = v.badge;
            badge.className = `fl-badge ${BADGE[v.tone]}`;
            r.dataset.tone = v.tone;
            const tick = r.querySelector("[data-mf-use]");
            tick.hidden = !v.tick;
            tick.checked = this.use[k];
            tick.disabled = this.importing;
            const dl = r.querySelector("[data-mf-dl]");
            dl.hidden = !v.download;
            dl.textContent = v.download ?? "Download";
            dl.disabled = models.busy();
            r.querySelector("[data-mf-skip]").hidden = !v.skip;
        }
        const list = this.root.querySelector("[data-mf-notes]");
        const ns = scan ? notes(scan) : [];
        list.hidden = ns.length === 0;
        list.replaceChildren(...ns.map((n) => {
            const li = document.createElement("li");
            li.textContent = n;
            return li;
        }));
        const line = this.root.querySelector("[data-mf-line]");
        line.textContent = this.line.text;
        line.classList.toggle("ok", this.line.ok === true);
        line.classList.toggle("err", this.line.ok === false);
        this.renderProgress();
        const go = this.root.querySelector("[data-mf-go]");
        go.hidden = !!this.result || !scan;
        go.disabled = !canUse(scan, this.use) || this.importing || models.busy();
        go.textContent = this.importing ? "Copying…" : "Use these";
        this.root.querySelector("[data-mf-pick]").disabled = this.importing;
        this.changed();
    }
    renderProgress() {
        const box = this.root.querySelector("[data-mf-progress]");
        const p = models.progress(JOB_ID);
        const on = this.importing && !!p && (p.state === "importing" || p.state === "verifying");
        box.hidden = !this.importing;
        const pct = on && p.total ? Math.round((p.received / p.total) * 100) : 0;
        const bar = box.querySelector("[data-mf-bar]");
        bar.setAttribute("aria-valuenow", String(pct));
        bar.firstElementChild.style.width = `${pct}%`;
        box.querySelector("[data-mf-progress-line]").textContent =
            on && p.state === "verifying" ? "Checking the fingerprint…" : `Copying ${on ? Math.round(p.received / 1e6) : 0} of ${mbText(on ? p.total : 0)} · from this PC`;
    }
    changed() {
        for (const l of this.listeners)
            l();
    }
}
function $folder(root) {
    return root.querySelector("[data-mf-folder]");
}
// ------------------------------------------------------------ Settings → Speech to text
/** Settings → Speech to text → Advanced: the same checklist in a dialog. */
export const FOLDER_ROW = `
<div class="fl-row" id="mf_row" data-needs="local">
  <div class="fl-row-text">
    <div class="fl-row-title">Model files on this PC</div>
    <div class="fl-row-desc">Point at one folder: the English, Hebrew and auto-detect models in it are found by their files, checked and set up.</div>
  </div>
  <div class="fl-row-control">
    <button type="button" class="fl-btn fl-btn-secondary" id="mf_open">Import from folder…</button>
  </div>
</div>`;
const DIALOG = `
<dialog class="fl-dialog lw-dl-dialog lw-mf-dialog" id="mf_dialog" aria-labelledby="mf_title">
  <div class="lw-dl-head">
    <h2 class="fl-serif lw-dl-title" id="mf_title">Import from folder</h2>
    <p class="lw-dl-lead">Models are recognised by their files, whatever the folders are called, and each file is checked against the fingerprint built into this version. Nothing is downloaded unless you press Download.</p>
  </div>
  ${folderChecklist("mf_settings")}
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-ghost" id="mf_close">Close</button>
  </div>
</dialog>`;
let settingsList = null;
async function openDialog() {
    const list = settingsList;
    if (!list || models.busy())
        return;
    if (!(await list.choose()))
        return;
    const d = $("mf_dialog");
    if (!d.open)
        d.showModal();
}
export const modelFolderFeature = {
    init() {
        document.body.append(html(DIALOG));
        settingsList = new FolderChecklist($("mf_settings"));
        const d = $("mf_dialog");
        const close = () => {
            if (settingsList?.busy)
                return;
            d.close();
            settingsList?.reset();
        };
        $("mf_close").addEventListener("click", close);
        d.addEventListener("cancel", (e) => {
            e.preventDefault();
            close();
        });
        settingsList.onChange(() => {
            $("mf_close").disabled = !!settingsList?.busy;
        });
        document.getElementById("mf_open")?.addEventListener("click", () => void openDialog());
    },
};
