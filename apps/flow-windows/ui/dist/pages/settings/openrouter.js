// Settings → OpenRouter: the account, what it is used for, privacy and
// this month's spend (features/openrouter.ts). Markup only; the speech and
// cleanup switches and models are on those pages.
import { group, heading, row, toggle } from "../../lib/markup.js";
export const settingsOpenrouterPage = {
    id: "settings-openrouter",
    area: "settings",
    template: `
${heading("OpenRouter", "The only place audio or text can reach a third party: openrouter.ai and the model provider it routes to. Off until you turn it on.")}
<ul id="or_warnings" class="lw-warnings"></ul>
<div class="fl-callout fl-callout-peach lw-section" id="or_notice_box" hidden>
  <div class="fl-grow"><div class="fl-callout-title">Cloud settings changed</div><div class="fl-callout-body" id="or_notice"></div></div>
  <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_notice_ok">OK</button>
</div>
${group("", row("API key", `<span id="or_key_state">No key saved</span>. Saved in Windows Credential Manager, never in settings files or logs; only ever sent to openrouter.ai.`, `<input id="or_key" class="fl-input" type="password" autocomplete="off" spellcheck="false" placeholder="sk-or-v1-…" />
     <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_key_paste">Paste</button>
     <button type="button" class="fl-btn fl-btn-primary fl-btn-sm" id="or_key_save">Save</button>
     <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="or_key_clear">Clear</button>`) +
        row("Credit", `<dl id="or_key_info" class="kv" hidden></dl><span id="or_key_warn" class="hint warn" hidden>This key has no credit limit; OpenRouter recommends setting one.</span> <span id="or_key_status"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_key_check">Check key</button>`))}
${group("Used for", row("Speech to text", `<span id="or_used_stt"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-go="settings-speech">Speech to text</button>`) +
        row("AI cleanup", `<span id="or_used_cleanup"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-go="settings-cleanup">AI cleanup</button>`))}
${group("Privacy", row("Zero-retention providers only", "Only providers that delete your request as soon as they answer. Recommended.", toggle("or_zdr", "Zero-retention providers only")) +
        row("Block providers that collect data", `Skips providers that may train on what you send. <span id="or_privacy_status"></span>`, toggle("or_dc_deny", "Block providers that collect data")))}
${group("This month", row(`<span id="or_usage_total"></span>`, `<ul id="or_usage_rows" class="lw-plain"></ul><span id="or_usage_status"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_reset_usage">Reset counters</button>`) +
        row("Monthly budget", "US dollars, shared by speech and cleanup. When the next request could pass it, everything switches to this PC until next month. Empty = no limit.", `<input id="or_budget" class="fl-input is-narrow" type="number" min="0" step="0.5" placeholder="no limit" />`))}

<dialog id="or_stt_consent_dlg" class="fl-dialog">
  <h3>Send audio to OpenRouter?</h3>
  <p>When on, the audio of your dictations leaves this PC and is sent to OpenRouter (openrouter.ai) and the model provider it routes to, along with your dictionary words. Routed only to zero-retention providers by default.</p>
  <p class="hint">You can turn this off at any time.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="or_stt_consent_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_stt_consent_ok">Send audio to OpenRouter</button>
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

<dialog id="or_confirm" class="fl-dialog">
  <h3 id="or_confirm_title"></h3>
  <p id="or_confirm_body"></p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="or_confirm_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="or_confirm_ok">OK</button>
  </div>
</dialog>

<dialog id="or_picker" class="fl-dialog picker">
  <h3 id="or_picker_title">Model</h3>
  <div class="lw-toolbar">
    <input id="or_picker_query" class="fl-input lw-grow" type="search" placeholder="Search by name or id" spellcheck="false" />
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_cat_refresh">Refresh models</button>
  </div>
  <span id="or_cat_status" class="hint"></span>
  <div class="chips">
    <span id="or_chip_text" class="lw-chip">Text</span>
    <button type="button" class="lw-chip toggle" id="or_chip_transcriptions" aria-pressed="false">Transcription</button>
    <button type="button" class="lw-chip toggle" id="or_chip_chat_audio" aria-pressed="false">Chat audio</button>
    <button type="button" class="lw-chip toggle" id="or_chip_zdr" aria-pressed="true">ZDR only</button>
    <button type="button" class="lw-chip toggle" id="or_chip_fast" aria-pressed="false">Fast providers</button>
    <button type="button" class="lw-chip toggle" id="or_chip_free" aria-pressed="false">Free</button>
  </div>
  <ul id="or_picker_list" class="models"></ul>
  <p id="or_picker_empty" class="hint" hidden>No models match. Turn off a filter or type a custom model id.</p>
  <div class="lw-toolbar">
    <input id="or_picker_custom" class="fl-input lw-grow" spellcheck="false" placeholder="Custom model id: author/model or author/model:nitro" aria-label="Custom model id" />
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_picker_use">Use</button>
  </div>
  <p id="or_picker_status" class="hint"></p>
  <div class="lw-dialog-actions"><button type="button" class="fl-btn fl-btn-secondary" id="or_picker_close">Close</button></div>
</dialog>`,
};
