// Settings → Your server (design: SettingsServer.dc.html): the flow-server
// connection (features/engines.ts validates it and saves it with Save).
// Markup only.
import { group, heading, row, toggle } from "../../lib/markup.js";
import { extLink, SERVER_HELP } from "../../features/models.js";
const err = (field) => `<small class="field-err" data-err="${field}"></small>`;
/** A preset timeout list; features/engines.ts adds the saved value when it is not a preset. */
const choice = (id, label, opts) => `<select id="dev_${id}" class="fl-select lw-w160" data-dev-field aria-label="${label}">${opts
    .map(([v, t]) => `<option value="${v}">${t}</option>`)
    .join("")}</select>`;
export const settingsServerPage = {
    id: "settings-server",
    area: "settings",
    template: `
${heading("Your server", `Run flow-server on a faster computer you own, at home or on a VPN like Tailscale, and this PC sends it the audio. It keeps nothing, and nothing goes to a third party. ${extLink(SERVER_HELP, "Set up a server →")}`)}
${group("Connection", row(`<label for="dev_host">Address</label>`, `A name or IP address, without ws:// or a port. ${err("host")}${err("port")}`, `<input id="dev_host" class="fl-input lw-w220" data-dev-field spellcheck="false" placeholder="100.64.0.10 or mybox.local" />
     <input id="dev_port" class="fl-input lw-w72" data-dev-field type="number" min="1" max="65535" aria-label="Port" />`) +
        row(`<label for="dev_token">Access token</label>`, `From <code>flow-server gen-token</code>. Stored encrypted for your Windows account; never logged. ${err("token")}`, `<input id="dev_token" class="fl-input lw-w180" data-dev-field type="password" autocomplete="off" spellcheck="false" placeholder="from flow-server gen-token" />
       <button type="button" class="fl-btn fl-btn-secondary" id="dev_token_paste">Paste</button>
       <span class="lw-row-extra">
         <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="dev_token_show">Show</button>
         <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="dev_token_clear">Clear</button>
       </span>`) +
        row("Encrypt the connection (TLS)", `Always on for public addresses.<span id="dev_tls_forced" hidden> Required for this address.</span>`, toggle("dev_tls", "Encrypt the connection (TLS)", "data-dev-field")) +
        row(`<label for="dev_tls_pin_sha256">Certificate fingerprint</label>`, `Optional. Pins this exact server. ${err("tls_pin_sha256")}`, `<input id="dev_tls_pin_sha256" class="fl-input lw-w300 lw-mono-input" data-dev-field spellcheck="false" placeholder="SHA-256" />`, `id="dev_pin_row"`))}
<div class="lw-actions lw-actions-tight">
  <button type="button" id="dev_save" class="fl-btn fl-btn-primary">Save</button>
  <button type="button" id="dev_test_conn" class="fl-btn fl-btn-outline">Test connection</button>
  <button type="button" id="srv_test_dictation" class="fl-btn fl-btn-outline">Test a dictation</button>
  <span id="dev_status" class="lw-status"></span>
</div>
<div id="dev_test" class="lw-result" role="status" hidden>
  <span class="lw-dot"></span><span class="lw-result-text" id="dev_test_text"></span>
</div>
${group("Users & keys", row("Keys for people and devices", `This app's key is an admin key, so you can give each person or device its own key and revoke it. <span id="srv_keys_who"></span>`, `<button type="button" class="fl-btn fl-btn-outline" data-go="settings-keys">Manage keys</button>`), `id="srv_keys" hidden`)}
${group("Advanced", row("Give up connecting after", `Then the next engine takes over. ${err("connect_timeout_ms")}`, choice("connect_timeout_ms", "Give up connecting after", [
        [500, "0.5 seconds"],
        [800, "0.8 seconds"],
        [1500, "1.5 seconds"],
        [3000, "3 seconds"],
        [5000, "5 seconds"],
    ])) +
        row("Wait for an answer up to", `Grows with the length of the dictation. ${err("commit_timeout_base_ms")}`, choice("commit_timeout_base_ms", "Wait for an answer up to", [
            [2000, "2 seconds"],
            [3000, "3 seconds"],
            [4000, "4 seconds"],
            [8000, "8 seconds"],
            [15000, "15 seconds"],
        ])) +
        row("Check the server every", err("health_ttl_s"), choice("health_ttl_s", "Check the server every", [
            [10, "10 seconds"],
            [30, "30 seconds"],
            [60, "1 minute"],
            [120, "2 minutes"],
            [300, "5 minutes"],
        ])))}`,
};
