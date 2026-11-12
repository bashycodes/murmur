// Settings → Cloud (D78; design: SettingsOpenRouter.dc.html "Settings ·
// Cloud", the model picker: ModelPicker.dc.html): one Cloud provider at a
// time, OpenRouter or the user's LiteLLM proxy (any OpenAI-compatible
// server, D52), its account or address, what it is used for, privacy and
// this month's spend (features/openrouter.ts). Markup only; the speech and
// cleanup switches and models are on those pages. `data-cloud` parts show
// for one provider only.

import { ICON, iconSvg } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";
import { extLink } from "../../features/models.js";

export const settingsOpenrouterPage: Page = {
  id: "settings-openrouter",
  area: "settings",
  template: `
${heading(
  "Cloud",
  "Bigger models that run somewhere else. Pick one provider. Nothing is sent until you turn on speech or cleanup below, and each asks for permission first.",
)}
<ul id="or_warnings" class="lw-warnings"></ul>
<div class="fl-callout fl-callout-peach lw-section" id="or_notice_box" hidden>
  <div class="fl-grow"><div class="fl-callout-title">Cloud settings changed</div><div class="fl-callout-body" id="or_notice"></div></div>
  <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_notice_ok">OK</button>
</div>
<section class="lw-section">
  <div class="fl-group lw-pad">
    <div class="lw-seg" role="radiogroup" aria-label="Cloud provider" id="cloud_provider_seg">
      <button type="button" role="radio" data-cloud-provider="openrouter">OpenRouter</button>
      <button type="button" role="radio" data-cloud-provider="custom">LiteLLM</button>
    </div>
    <p class="fl-row-desc" id="cloud_provider_desc"></p>
    <p class="fl-row-desc" id="cloud_provider_offers"></p>
  </div>
</section>
${group(
  "Your LiteLLM proxy",
  row(
    "Address",
    `Any OpenAI-compatible server works. We add <code class="lw-mono">/v1</code> if you leave it off. <span id="cu_url_norm" class="hint lw-mono"></span>`,
    `<input id="cu_base_url" class="fl-input lw-w220 lw-mono-input" spellcheck="false" autocomplete="off" placeholder="http://192.168.1.20:4000" aria-label="Proxy address" />`,
  ) +
    row(
      "API key",
      `<span id="cu_key_state" class="lw-key-state">No key saved</span> Optional. Saved in Windows Credential Manager, sent only to this address.`,
      `<span id="cu_key_saved_ctl" hidden><button type="button" class="fl-btn fl-btn-ghost" id="cu_key_clear">Remove</button></span>
       <span id="cu_key_entry"><input id="cu_key" class="fl-input lw-w160 lw-mono-input" type="password" autocomplete="off" spellcheck="false" placeholder="sk-…" aria-label="API key (optional)" />
       <button type="button" class="fl-btn fl-btn-primary" id="cu_key_save">Save</button></span>`,
    ) +
    row(
      "Connection",
      `<span id="cu_test_out"></span>`,
      `<button type="button" class="fl-btn fl-btn-secondary" id="cu_test">Test connection</button>`,
    ) +
    row(
      "Speech model",
      "For speech to text on your proxy. Test connection lists the ones it has and picks the first.",
      `<select id="cu_stt_model" class="fl-select lw-w220" aria-label="Speech model">
        <option value="">None: speech stays on this PC and your server</option>
      </select>`,
    ) +
    `<div class="fl-callout fl-callout-peach" id="cu_http_warn" hidden><div class="fl-grow"><div class="fl-callout-body">This address starts with http, so your key and your words cross the network unencrypted. Use https if your proxy supports it.</div></div></div>`,
  `data-cloud="custom"`,
)}
${group(
  "API key",
  row(
    `<span id="or_key_state" class="lw-key-state">No key saved</span>`,
    "Saved in Windows Credential Manager. Never written to settings files or logs; only ever sent to openrouter.ai.",
    `<span class="lw-key-saved" id="or_key_saved_ctl" hidden>
       <button type="button" class="fl-btn fl-btn-ghost" id="or_key_clear">Remove</button>
       <button type="button" class="fl-btn fl-btn-secondary" id="or_key_check">Check key</button>
     </span>`,
  ) +
    row(
      `<label for="or_key" id="or_key_label">Paste a key</label>`,
      `From ${extLink("https://openrouter.ai/settings/keys", "openrouter.ai → Keys")}. Set a credit limit on the key.`,
      `<input id="or_key" class="fl-input lw-w220 lw-mono-input" type="password" autocomplete="off" spellcheck="false" placeholder="sk-or-v1-…" />
       <button type="button" class="fl-btn fl-btn-secondary" id="or_key_paste">Paste</button>
       <button type="button" class="fl-btn fl-btn-primary" id="or_key_save">Save</button>`,
      `id="or_key_entry"`,
    ) +
    row(
      "Credit",
      `<span id="or_key_info" hidden></span><span id="or_key_warn" class="hint warn" hidden> This key has no credit limit; OpenRouter recommends setting one.</span> <span id="or_key_status"></span>`,
      `<span class="fl-badge fl-badge-light" id="or_key_badge">Not checked</span>`,
      `id="or_credit_row"`,
    ),
  `data-cloud="openrouter"`,
)}
${group(
  "Used for",
  row(
    "Speech to text",
    `<span id="or_used_stt"></span>`,
    `<span class="fl-badge fl-badge-light" id="or_used_stt_badge">Off</span><button type="button" class="lw-link" data-go="settings-speech" id="or_used_stt_link">Set up →</button>`,
  ) +
    row(
      "AI cleanup",
      `<span id="or_used_cleanup"></span>`,
      `<span class="fl-badge fl-badge-light" id="or_used_cleanup_badge">Off</span><button type="button" class="lw-link" data-go="settings-cleanup" id="or_used_cleanup_link">Set up →</button>`,
    ),
)}
${group(
  "Privacy",
  row(
    `Sent only to <span id="cloud_ll_host" class="lw-mono"></span>`,
    `<span id="cloud_ll_consent"></span> Pointing the address at another server asks again. Where the proxy sends it next is up to how you set it up.`,
    "",
    `data-cloud="custom"`,
  ) +
    row(
      "Zero-retention providers only",
      "Only providers that delete your request as soon as they answer. Recommended.",
      toggle("or_zdr", "Zero-retention providers only"),
      `data-cloud="openrouter"`,
    ) +
    row(
      "Block providers that collect data",
      `Skips providers that may train on what you send. <span id="or_privacy_status"></span>`,
      toggle("or_dc_deny", "Block providers that collect data"),
      `data-cloud="openrouter"`,
    ),
)}
<section class="lw-section">
  <h3 class="fl-group-title">This month</h3>
  <div class="fl-group">
    <div class="fl-row lw-usage">
      <div class="lw-usage-head"><span class="fl-serif lw-usage-total" id="or_usage_amount">$0</span><span class="lw-usage-of" id="or_usage_of"></span></div>
      <div class="lw-meter" id="or_usage_meter" hidden><span id="or_usage_bar"></span></div>
      <div class="lw-usage-split"><span id="or_usage_total"></span></div>
      <ul id="or_usage_rows" class="lw-plain lw-usage-rows"></ul>
      <p class="fl-row-desc" data-cloud="custom">Shown when your proxy reports a cost. Your own proxy has no budget here; the OpenRouter budget never counts it.</p>
    </div>
    ${row(
      "Monthly budget",
      `Shared by speech and cleanup. When the next request could pass it, everything switches to this PC until next month. <span id="or_usage_status"></span>`,
      `<select id="or_budget" class="fl-select lw-w160" aria-label="Monthly budget">
        <option value="">No limit</option>
        <option value="2">$2 a month</option>
        <option value="5">$5 a month</option>
        <option value="10">$10 a month</option>
        <option value="20">$20 a month</option>
      </select>`,
      `data-cloud="openrouter"`,
    )}
    ${row(
      "Reset counters",
      "",
      `<button type="button" class="fl-btn fl-btn-secondary" id="or_reset_usage">Reset</button>`,
    )}
  </div>
</section>

<dialog id="or_stt_consent_dlg" class="fl-dialog">
  <h3>Send audio to OpenRouter?</h3>
  <p>When on, the audio of your dictations leaves this PC and is sent to OpenRouter (openrouter.ai) and the model provider it routes to, along with your dictionary words. Routed only to zero-retention providers by default.</p>
  <p class="hint">You can turn this off at any time.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="or_stt_consent_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_stt_consent_ok">Send audio to OpenRouter</button>
  </div>
</dialog>

<dialog id="cu_stt_consent_dlg" class="fl-dialog">
  <h3>Send audio to <span id="cu_stt_consent_host_title"></span>?</h3>
  <p>When on, the audio of your dictations leaves this PC and is sent to your own proxy at <strong id="cu_stt_consent_host"></strong>, along with your dictionary words, and to whatever model it forwards to.</p>
  <p class="hint warn" id="cu_stt_consent_http" hidden>This address uses http://: the key and the audio cross the network unencrypted.</p>
  <p class="hint">Pointing the address at another server asks again. You can turn this off at any time.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="cu_stt_consent_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="cu_stt_consent_ok">Send audio to this proxy</button>
  </div>
</dialog>

<dialog id="or_cleanup_consent_dlg" class="fl-dialog">
  <h3>Send dictated text to OpenRouter?</h3>
  <p>After each dictation, the text (and your dictionary words) is sent to OpenRouter (openrouter.ai) and to the model provider it routes to. Your voice is not sent unless OpenRouter speech to text is also on. Routed only to zero-retention providers by default.</p>
  <p class="hint">You can turn this off at any time.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="or_cleanup_consent_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_cleanup_consent_ok">Send text to OpenRouter</button>
  </div>
</dialog>

<dialog id="cu_consent_dlg" class="fl-dialog">
  <h3>Send dictated text to <span id="cu_consent_host_title"></span>?</h3>
  <p>After each dictation, the text (and your dictionary words) is sent to your own proxy at <strong id="cu_consent_host"></strong>, and to whatever model it forwards to. Your voice is not sent.</p>
  <p class="hint warn" id="cu_consent_http" hidden>This address uses http://: the key and the text cross the network unencrypted.</p>
  <p class="hint">Pointing the address at another server asks again. You can turn this off at any time.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="cu_consent_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="cu_consent_ok">Send text to this proxy</button>
  </div>
</dialog>

<dialog id="or_confirm" class="fl-dialog">
  <h3 id="or_confirm_title"></h3>
  <p id="or_confirm_body"></p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="or_confirm_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_confirm_ok">OK</button>
  </div>
</dialog>

<dialog id="or_picker" class="fl-dialog lw-picker" aria-labelledby="or_picker_title">
  <div class="lw-picker-head">
    <div class="lw-grow">
      <h2 class="fl-serif lw-picker-title" id="or_picker_title">Choose a model</h2>
      <div class="lw-picker-sub"><span id="or_cat_status" class="hint"></span> <button type="button" class="lw-link lw-link-sm" id="or_cat_refresh">Refresh</button></div>
    </div>
    <button type="button" class="fl-iconbtn" id="or_picker_close" aria-label="Close">${iconSvg(ICON.close)}</button>
  </div>
  <div class="lw-picker-filters">
    <div class="lw-search-field">
      ${iconSvg(ICON.search, 16)}
      <input id="or_picker_query" class="fl-input" type="search" placeholder="Search models" aria-label="Search models" spellcheck="false" />
    </div>
    <div class="chips">
      <button type="button" class="lw-chip" id="or_chip_transcriptions" aria-pressed="false">Transcription</button>
      <button type="button" class="lw-chip" id="or_chip_chat_audio" aria-pressed="false">Chat models with audio</button>
      <button type="button" class="lw-chip" id="or_chip_zdr" aria-pressed="true">Zero retention only</button>
      <button type="button" class="lw-chip" id="or_chip_fast" aria-pressed="false">Fast providers</button>
      <button type="button" class="lw-chip" id="or_chip_free" aria-pressed="false">Free</button>
    </div>
  </div>
  <div id="or_picker_list" class="fl-list lw-picker-list" role="listbox" aria-label="Models"></div>
  <p id="or_picker_empty" class="hint" hidden>No models match. Turn off a filter or paste a model ID.</p>
  <p id="or_picker_hidden" class="hint" hidden></p>
  <div class="lw-picker-foot">
    <input id="or_picker_custom" class="fl-input lw-grow lw-mono-input" spellcheck="false" placeholder="Or paste a model ID" aria-label="Custom model ID: author/model or author/model:nitro" />
    <button type="button" class="fl-btn fl-btn-ghost" id="or_picker_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_picker_use">Use this model</button>
  </div>
  <p id="or_picker_status" class="hint"></p>
</dialog>`,
};
