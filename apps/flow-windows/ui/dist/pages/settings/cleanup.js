// Settings → AI cleanup (design: SettingsCleanup.dc.html): the cleanup
// level and formatting (features/ai.ts, features/general.ts), the cleanup
// order (D36: this PC, your server, OpenRouter; features/ai.ts) and
// cloud cleanup: OpenRouter or, D52, the user's own OpenAI-compatible
// server (features/openrouter.ts). Markup only.
import { ICON } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import { browseButton, extLink, LLM_DOWNLOAD_ROW } from "../../features/models.js";
import { engineRow } from "./speech.js";
/** A small switch with its label, for a stacked list of switches in one row. */
const small = (id, label) => `<label class="lw-small-switch">${toggle(id, label).replace('class="fl-toggle"', 'class="fl-toggle is-small"')}${label}</label>`;
export const settingsCleanupPage = {
    id: "settings-cleanup",
    area: "settings",
    template: `
${heading("AI cleanup", `Turns what you said into what you meant: punctuation, paragraphs, lists, self-corrections and tone. How it sounds in each app is set in <button type="button" class="lw-link" data-go="style">Style →</button>`)}
<section class="lw-section">
  <h3 class="fl-group-title">How much to change</h3>
  <div class="fl-group lw-pad">
    <div class="lw-seg" role="radiogroup" aria-label="Cleanup level" id="level_seg">
      <button type="button" role="radio" data-level="verbatim">Verbatim</button>
      <button type="button" role="radio" data-level="none">Rules only</button>
      <button type="button" role="radio" data-level="light">Light</button>
      <button type="button" role="radio" data-level="medium">Medium</button>
      <button type="button" role="radio" data-level="high">High</button>
    </div>
    <select id="cleanup_level" data-save="prefs" hidden>
      <option value="verbatim">Verbatim</option>
      <option value="none">Rules only</option>
      <option value="light">Light</option>
      <option value="medium">Medium</option>
      <option value="high">High</option>
    </select>
    <div class="lw-level-text">
      <div class="fl-row-title" id="level_title"></div>
      <div class="fl-row-desc" id="level_desc"></div>
    </div>
    <div class="lw-samples" aria-label="Example">
      <div class="lw-sample"><div class="fl-overline">You said</div><div class="lw-sample-said">um so the meeting is on tuesday no wait thursday at 3 and uh bring the the slides</div></div>
      <div class="lw-sample"><div class="fl-overline">Pasted</div><div class="lw-sample-out" id="level_sample"></div></div>
    </div>
  </div>
</section>

<section class="lw-section" id="cleanup_order_group">
  <h3 class="fl-group-title">Cleanup order</h3>
  <div class="fl-group" id="ai_routes">
    ${engineRow("aieng", "cleanup", "local", ICON.chip, "This PC", toggle("llm_enabled", "Use this PC for cleanup", `data-save="prefs"`), `data-needs="local"`)}
    ${engineRow("aieng", "cleanup", "server", ICON.server, "Your server", toggle("aieng_server_on", "Use your server for cleanup"), `data-needs="remote"`)}
    ${engineRow("aieng", "cleanup", "cloud", ICON.cloud, "OpenRouter", toggle("or_cleanup_allow", "Use OpenRouter for cleanup"), `data-needs="cloud"`)}
    <div class="fl-row lw-foot-row"><div class="fl-row-desc">Every dictation is cleaned by the first engine that is on and ready, whichever engine transcribed it; if it fails or runs out of time, the next one tries, and the built-in formatting rules finish the job. Now: <span id="ai_engine_line">On this PC</span>. <span id="or_cleanup_consent"></span> <span id="cleanup_order_msg" class="hint"></span></div></div>
  </div>
</section>

<section class="lw-section" id="or_cleanup_group" data-needs="cloud">
  <h3 class="fl-group-title" id="or_cleanup_title">Cloud server</h3>
  <div class="fl-group" id="or_cleanup_box">
    ${row("Send text to", "Cloud speech to text always uses OpenRouter.", `<select id="or_cleanup_provider" class="fl-select lw-w220" aria-label="Cloud cleanup provider">
        <option value="openrouter">OpenRouter</option>
        <option value="custom">Custom server (LiteLLM / OpenAI-compatible)</option>
      </select>`)}
    ${row("Server address", `The base URL of your LiteLLM proxy or other OpenAI-compatible server. <code class="lw-mono">/v1</code> is added when no path is given; include the full path if your server uses a prefix. <span id="cu_url_norm" class="hint lw-mono"></span> <span id="cu_http_warn" class="hint warn" hidden>This is http://: the key and your dictated text are sent unencrypted.</span>`, `<input id="cu_base_url" class="fl-input lw-w220 lw-mono-input" spellcheck="false" autocomplete="off" placeholder="http://192.168.1.20:4000" aria-label="Server base URL" />`, `data-provider="custom"`)}
    ${row("API key", `<span id="cu_key_state" class="lw-key-state">No key saved</span> Optional. Saved in Windows Credential Manager and only ever sent to this server.`, `<span id="cu_key_saved_ctl" hidden><button type="button" class="fl-btn fl-btn-ghost" id="cu_key_clear">Remove</button></span>
       <span id="cu_key_entry"><input id="cu_key" class="fl-input lw-w160 lw-mono-input" type="password" autocomplete="off" spellcheck="false" placeholder="sk-…" aria-label="API key (optional)" />
       <button type="button" class="fl-btn fl-btn-primary" id="cu_key_save">Save</button></span>`, `data-provider="custom"`)}
    ${row("Model", `Any model your server serves; type it or load the list. <span id="cu_models_out"></span>`, `<input id="cu_model" class="fl-input lw-w220 lw-mono-input" list="cu_model_list" spellcheck="false" autocomplete="off" placeholder="gpt-4o-mini" aria-label="Model id" />
       <datalist id="cu_model_list"></datalist>
       <button type="button" class="fl-btn fl-btn-secondary" id="cu_models_load">Load models</button>`, `data-provider="custom"`)}
    ${row("Give up after", "Then this PC takes over.", `<select id="cu_timeout_ms" class="fl-select lw-w160">
        <option value="2500">2.5 seconds</option>
        <option value="4000">4 seconds</option>
        <option value="8000">8 seconds</option>
        <option value="15000">15 seconds</option>
        <option value="30000">30 seconds</option>
      </select>`, `data-provider="custom"`)}
    ${row("Connection", `<span id="cu_test_out"></span>`, `<button type="button" class="fl-btn fl-btn-secondary" id="cu_test">Test connection</button>`, `data-provider="custom"`)}
    ${row(`<span id="or_cleanup_model_name"></span> <span id="or_cleanup_model_badges" class="pills"></span>`, `<span id="or_cleanup_model_desc"></span> <code id="or_cleanup_model_id" class="lw-mono"></code>`, `<button type="button" class="fl-btn fl-btn-secondary lw-btn-160" id="or_cleanup_change">Change</button>`, `data-provider="openrouter"`)}
    ${row("Prefer providers that are", "", `<select id="or_cleanup_sort" class="fl-select lw-w160">
        <option value="latency">Fastest</option>
        <option value="price">Cheapest</option>
        <option value="throughput">Highest throughput</option>
        <option value="">OpenRouter default</option>
      </select>`, `data-provider="openrouter"`)}
    ${row("Give up after", "Then this PC takes over.", `<select id="or_cleanup_timeout_ms" class="fl-select lw-w160">
        <option value="1500">1.5 seconds</option>
        <option value="2500">2.5 seconds</option>
        <option value="5000">5 seconds</option>
        <option value="8000">8 seconds</option>
      </select>`, `data-provider="openrouter"`)}
    ${row("Also send", "Your words, the app type, the writing style and your dictionary are always sent. More context gives better results.", `<div class="lw-switch-stack">
        ${small("or_cleanup_send_surrounding_text", "Text around the cursor")}
        ${small("or_cleanup_send_screen_words", "Words on screen")}
        ${small("or_cleanup_send_app_name", "App name")}
      </div>`)}
    ${row("Try it", `<span id="or_cleanup_test_out"></span> <span id="or_cleanup_state"></span> <span id="or_cleanup_msg"></span>`, `<button type="button" class="fl-btn fl-btn-secondary" id="or_cleanup_test">Test</button>`)}
  </div>
</section>

${group("On this PC", row("Model file", `A GGUF model. Qwen2.5-1.5B-Instruct Q4_K_M is the default; Qwen2.5-0.5B-Instruct is about twice as fast but misses many fixes (${extLink("https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF", "get it →")}, or download one below). Never downloaded automatically. Empty = default.`, `<input id="llm_model_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />${browseButton("llm_model_path")}`, `data-needs="local"`) +
        LLM_DOWNLOAD_ROW +
        row("Engine library", `llama.cpp, <code id="llm_plugin" class="lw-mono"></code>. Empty = next to the app.`, `<input id="llm_lib_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />`, `data-needs="local"`) +
        row("Give up after", "Then the formatting rules take over.", `<select id="llm_timeout_ms" class="fl-select lw-w160" data-save="prefs">
        <option value="1500">1.5 seconds</option>
        <option value="3000">3 seconds</option>
        <option value="6000">6 seconds</option>
        <option value="10000">10 seconds</option>
      </select>`, `data-needs="local"`) +
        row("Use what's on screen", "Reads nearby text in the app to match names and context. Stays on this PC.", toggle("use_ax_context", "Use what's on screen", `data-save="prefs"`)) +
        row("Formatting rules", `Punctuation, quotes, fillers and corrections: "comma", "new line", "quote … unquote", "um", "scratch that", "$5", "3pm". Run on this PC with or without AI. Off = exactly what you said (same as Verbatim).`, toggle("spoken_cleanup", "Formatting rules", `data-save="prefs"`)))}`,
};
