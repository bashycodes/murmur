// Settings → Users & keys (D39): the server's named keys (D38), shown only
// when this app's saved server token is an admin key (`admin_status`).
// List with usage, create (the secret is shown once in a copy box and
// dropped from the page when closed), revoke with a confirm. Key names and
// clients are user data: always textContent, never markup.
import { $, askDialog, el, setStatus, when } from "../../lib/dom.js";
import { invoke } from "../../lib/ipc.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import { router } from "../../lib/router.js";
import { views } from "../../lib/views.js";
const listeners = [];
let status = { configured: false, admin: false, name: "", id: "" };
let checking = 0;
/** The last `admin_status` (the shell hides the page unless `admin`). */
export const adminStore = {
    get: () => status,
    onChange(l) {
        listeners.push(l);
    },
    /** Ask the server who this app's key is. Only the latest answer counts. */
    async refresh() {
        const n = ++checking;
        let s;
        try {
            s = await invoke("admin_status");
        }
        catch (e) {
            s = { configured: false, admin: false, name: "", id: "", error: String(e) };
        }
        if (n === checking) {
            status = s;
            for (const l of listeners)
                l(s);
        }
        return s;
    },
};
const seen = (secs) => (secs ? when(secs * 1000) : "never");
const day = (secs) => new Date(secs * 1000).toLocaleDateString([], { dateStyle: "medium" });
const minutes = (ms) => (ms / 60000).toFixed(1);
const megabytes = (b) => `${(b / 1e6).toFixed(1)} MB`;
export const settingsKeysPage = {
    id: "settings-keys",
    area: "settings",
    template: `
${heading("Users & keys", "Give each person or device its own key, so you can see who uses the server and switch one off without touching the others. Revoking a key stops its next connection; a dictation already running finishes.")}
<div id="keys_err" class="lw-result" role="status" hidden>
  <span class="lw-dot"></span><span class="lw-result-text" id="keys_err_text"></span>
</div>
<section class="lw-section">
  <h3 class="fl-group-title">Keys</h3>
  <div class="fl-group">
    <table class="lw-keys" id="keys_table">
      <thead><tr><th>Name</th><th>Created</th><th title="Last dictation or model download">Last used</th><th title="Last request of any kind, including connection checks and this page">Last connected</th><th class="num">Dictations</th><th class="num">Audio min</th><th class="num">Downloads</th><th></th></tr></thead>
      <tbody id="keys_rows"></tbody>
    </table>
    <p class="lw-empty-row" id="keys_empty" hidden>Loading…</p>
  </div>
</section>
${group("New key", row(`<label for="keys_name">Name</label>`, `Who or what it is for, like <code>phone</code> or <code>sams-laptop</code>. Letters, digits, <code>.</code> <code>_</code> <code>-</code>.`, `<input id="keys_name" class="fl-input lw-w220" maxlength="64" spellcheck="false" autocomplete="off" placeholder="phone" />`) +
        row("Admin", "An admin key can open this page and create or revoke keys. Give it only to devices you own.", toggle("keys_admin", "Admin")))}
<div class="lw-actions lw-actions-tight">
  <button type="button" id="keys_create" class="fl-btn fl-btn-primary">Create key</button>
  <button type="button" id="keys_refresh" class="fl-btn fl-btn-outline">Refresh</button>
  <span id="keys_status" class="lw-status"></span>
</div>

<dialog id="keys_secret_dlg" class="fl-dialog" aria-labelledby="keys_secret_title">
  <h3 id="keys_secret_title">Key created</h3>
  <p>Copy it now: this is the only time it is shown. Paste it into the app on <b id="keys_secret_name"></b> as the access token.</p>
  <div class="lw-secret">
    <input id="keys_secret" class="fl-input lw-mono-input" readonly spellcheck="false" autocomplete="off" aria-label="The new key" />
    <button type="button" class="fl-btn fl-btn-secondary" id="keys_secret_copy">Copy</button>
  </div>
  <p class="hint" id="keys_secret_copied" hidden>Copied.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-primary" id="keys_secret_done">Done</button>
  </div>
</dialog>

<dialog id="keys_revoke_dlg" class="fl-dialog" aria-labelledby="keys_revoke_title">
  <h3 id="keys_revoke_title"></h3>
  <p>Its next connection is refused. A dictation already running finishes. This can't be undone: to let it back in, create a new key.</p>
  <p class="lw-warn" id="keys_revoke_self" hidden>This is the key this app uses. After revoking it, this app can't reach the server until you save another key in Your server.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="keys_revoke_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-destructive" id="keys_revoke_ok">Revoke</button>
  </div>
</dialog>`,
    init() {
        $("keys_create").addEventListener("click", () => void create());
        $("keys_refresh").addEventListener("click", () => void load());
        $("keys_name").addEventListener("keydown", (e) => {
            if (e.key === "Enter")
                void create();
        });
        $("keys_secret_copy").addEventListener("click", () => void copySecret());
        const dlg = $("keys_secret_dlg");
        $("keys_secret_done").addEventListener("click", () => dlg.close());
        // However it closes (Done, Esc), the secret leaves the page.
        dlg.addEventListener("close", () => {
            $("keys_secret").value = "";
            $("keys_secret_copied").hidden = true;
        });
    },
    show(opened) {
        if (opened)
            void load();
    },
};
function showError(msg) {
    $("keys_err").hidden = !msg;
    $("keys_err").classList.toggle("is-err", !!msg);
    $("keys_err_text").textContent = msg ?? "";
}
async function load() {
    const empty = $("keys_empty");
    if (!$("keys_rows").childElementCount) {
        empty.textContent = "Loading…";
        empty.hidden = false;
    }
    try {
        render((await invoke("admin_list_keys")).keys);
        showError(null);
    }
    catch (e) {
        showError(String(e));
        if (!$("keys_rows").childElementCount)
            empty.textContent = "Couldn't load the keys.";
        void adminStore.refresh();
    }
}
// Model downloads: the count, with the bytes sent on hover.
function downloads(k) {
    const n = k.downloads ?? 0;
    const td = el("td", "num", String(n));
    if (n)
        td.title = megabytes(k.download_bytes ?? 0);
    return td;
}
function render(keys) {
    const body = $("keys_rows");
    body.replaceChildren();
    // Active first (oldest first, as the server lists them), then revoked.
    const active = keys.filter((k) => !k.revoked);
    const sorted = [...active, ...keys.filter((k) => k.revoked)];
    const admins = active.filter((k) => k.admin).length;
    for (const k of sorted) {
        const tr = el("tr");
        tr.dataset.keyId = k.id;
        if (k.revoked)
            tr.classList.add("is-revoked");
        const name = el("td", "name");
        name.append(el("span", "lw-key-name", k.name));
        const tags = el("span", "lw-key-tags");
        if (k.admin)
            tags.append(el("span", "fl-badge fl-badge-light", "admin"));
        if (k.current)
            tags.append(el("span", "fl-badge fl-badge-brandLight", "this app"));
        if (tags.childElementCount)
            name.append(tags);
        // The app it last connected with, under the name.
        name.append(el("span", "lw-key-client", k.client ?? "not used yet"));
        // A key used before the server tracked last use has counts but no time.
        const used = k.last_used || !(k.dictations || k.downloads) ? seen(k.last_used) : "unknown";
        // Older servers send only `last_seen`, which is last connected.
        const connected = seen(k.last_connected ?? k.last_seen);
        tr.append(name, el("td", "", day(k.created)), el("td", "", used), el("td", "", connected), el("td", "num", String(k.dictations)), el("td", "num", minutes(k.audio_ms)), downloads(k));
        const act = el("td", "act");
        if (k.revoked) {
            act.append(el("span", "lw-muted", `Revoked ${day(k.revoked)}`));
            act.classList.add("is-revoked");
        }
        else {
            const b = el("button", "fl-btn fl-btn-ghost fl-btn-sm", "Revoke");
            b.type = "button";
            b.dataset.revoke = k.id;
            if (k.admin && admins <= 1) {
                b.disabled = true;
                b.title = "The last admin key can't be revoked: create another admin key first";
            }
            b.addEventListener("click", () => void revoke(k));
            act.append(b);
        }
        tr.append(act);
        body.append(tr);
    }
    const empty = $("keys_empty");
    empty.hidden = keys.length > 0;
    empty.textContent = "No keys yet.";
}
async function create() {
    const name = $("keys_name").value.trim();
    const admin = $("keys_admin").checked;
    if (!name) {
        setStatus("keys_status", "Give the key a name", false);
        return;
    }
    const btn = $("keys_create");
    btn.disabled = true;
    try {
        const c = await invoke("admin_create_key", { name, admin });
        $("keys_name").value = "";
        $("keys_admin").checked = false;
        setStatus("keys_status", `Created ${c.key.name}`, true);
        showSecret(c);
        void load();
    }
    catch (e) {
        setStatus("keys_status", String(e), false);
    }
    finally {
        btn.disabled = false;
    }
}
function showSecret(c) {
    $("keys_secret_name").textContent = c.key.name;
    const input = $("keys_secret");
    input.value = c.secret;
    $("keys_secret_dlg").showModal();
    input.select();
}
async function copySecret() {
    const v = $("keys_secret").value;
    if (!v)
        return;
    try {
        await invoke("clipboard_copy", { text: v });
        $("keys_secret_copied").hidden = false;
    }
    catch (e) {
        $("keys_secret_copied").textContent = String(e);
        $("keys_secret_copied").hidden = false;
    }
}
async function revoke(k) {
    $("keys_revoke_title").textContent = `Revoke ${k.name}?`;
    $("keys_revoke_self").hidden = !k.current;
    if (!(await askDialog("keys_revoke_dlg", "keys_revoke_ok", "keys_revoke_cancel")))
        return;
    try {
        await invoke("admin_revoke_key", { id: k.id });
        setStatus("keys_status", `Revoked ${k.name}`, true);
    }
    catch (e) {
        setStatus("keys_status", String(e), false);
    }
    await load();
    // Revoking this app's own key ends its admin access.
    if (k.current)
        void adminStore.refresh();
}
// The engine settings loaded or were saved (a new server or token, maybe):
// ask again who this app's key is. Saves are rare; this is one request.
views.onDev((v) => {
    if (v.caps.remote)
        void adminStore.refresh();
});
// Opening Your server checks again (the server may have come up since).
router.onNavigate((id, prev) => {
    if (id === "settings-server" && prev !== id && views.dev?.caps.remote)
        void adminStore.refresh();
});
