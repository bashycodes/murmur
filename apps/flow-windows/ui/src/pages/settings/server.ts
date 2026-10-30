// Settings → Your server: the flow-server connection (features/engines.ts
// validates and saves it with an explicit Save). Markup only.

import { group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";

const err = (field: string): string => `<small class="field-err" data-err="${field}"></small>`;
const num = (id: string, min: number, max: number, step = 1): string =>
  `<input id="dev_${id}" class="fl-input is-narrow" data-dev-field type="number" min="${min}" max="${max}" step="${step}" />`;

export const settingsServerPage: Page = {
  id: "settings-server",
  area: "settings",
  template: `
${heading(
  "Your server",
  "Your own flow-server, on your network or a VPN. Audio goes only there, never to a third party.",
)}
${group(
  "Connection",
  row(
    "Address",
    `A name or IP address, without ws:// or a port. ${err("host")}`,
    `<input id="dev_host" class="fl-input is-wide" data-dev-field spellcheck="false" placeholder="100.64.0.10 or mybox.local" />`,
  ) +
    row("Port", err("port"), `<input id="dev_port" class="fl-input is-narrow" data-dev-field type="number" min="1" max="65535" />`) +
    row(
      "Access token",
      `From <code>flow-server gen-token</code>. Stored encrypted for your Windows account (DPAPI); never logged. ${err("token")}`,
      `<input id="dev_token" class="fl-input" data-dev-field type="password" autocomplete="off" spellcheck="false" placeholder="from flow-server gen-token" />
       <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dev_token_show">Show</button>
       <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dev_token_paste">Paste</button>
       <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="dev_token_clear">Clear</button>`,
    ) +
    row(
      "Encrypt the connection (TLS)",
      `Always on for public addresses.<span id="dev_tls_forced" hidden> Required for this address.</span>`,
      toggle("dev_tls", "Encrypt the connection (TLS)", "data-dev-field"),
    ) +
    row(
      "Certificate fingerprint",
      `Optional. SHA-256, pins this exact server. ${err("tls_pin_sha256")}`,
      `<input id="dev_tls_pin_sha256" class="fl-input is-wide" data-dev-field spellcheck="false" placeholder="64 hex characters" />`,
      `id="dev_pin_row"`,
    ),
)}
${group(
  "Advanced",
  row("Give up connecting after", `Milliseconds. Then the next engine takes over. ${err("connect_timeout_ms")}`, num("connect_timeout_ms", 100, 10000, 100)) +
    row(
      "Wait for an answer up to",
      `Milliseconds; grows with the length of the dictation. ${err("commit_timeout_base_ms")}`,
      num("commit_timeout_base_ms", 500, 60000, 100),
    ) +
    row("Check the server every", `Seconds. ${err("health_ttl_s")}`, num("health_ttl_s", 5, 600)),
)}
<div class="lw-actions">
  <button type="button" id="dev_save" class="fl-btn fl-btn-primary">Save</button>
  <button type="button" id="dev_test_conn" class="fl-btn fl-btn-secondary">Test connection</button>
  <span id="dev_status" class="lw-status"></span>
</div>
<p id="dev_test" class="hint"></p>`,
};
