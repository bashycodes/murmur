// Settings → Developer: engine status and diagnostics (features/engines.ts),
// and where the AI cleanup engine library loads from (features/ai.ts).
// Shown in builds with a server engine, or after seven taps on the version.

import { group, heading, row } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";

export const settingsDeveloperPage: Page = {
  id: "settings-developer",
  area: "settings",
  template: `
${heading("Developer")}
<section class="lw-section">
  <h3 class="fl-group-title">Status</h3>
  <div class="fl-group lw-pad">
    <dl class="kv">
      <dt>Build</dt><dd id="dev_build">…</dd>
      <dt>Last dictation</dt><dd id="dev_last">No dictation yet</dd>
      <dt>Server</dt><dd id="dev_server_info">Not probed yet</dd>
      <dt>Circuit</dt><dd id="dev_circuit">closed</dd>
      <dt id="dev_cloud_circuit_dt" hidden>Cloud circuit</dt><dd id="dev_cloud_circuit" hidden>closed</dd>
    </dl>
  </div>
</section>
<div class="lw-actions">
  <button type="button" id="dev_test_session" class="fl-btn fl-btn-secondary">Test session</button>
  <button type="button" id="dev_reset_circuit" class="fl-btn fl-btn-secondary">Reset circuit</button>
  <button type="button" id="dev_reset" class="fl-btn fl-btn-secondary lw-danger">Reset engines to defaults</button>
  <button type="button" id="dev_lock" class="fl-btn fl-btn-ghost" hidden>Lock</button>
</div>
<p class="lw-status-line"><span id="dev_diag_status"></span></p>
<p id="dev_diag" class="hint"></p>
${group(
  "AI cleanup on this PC",
  row(
    `<label for="llm_lib_path">Engine library</label>`,
    `llama.cpp, <code id="llm_plugin" class="lw-mono"></code>. Empty = next to the app.`,
    `<input id="llm_lib_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />`,
  ),
  `data-needs="local"`,
)}`,
};
