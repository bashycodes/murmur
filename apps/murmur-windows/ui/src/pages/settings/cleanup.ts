// Settings → AI cleanup (design: SettingsCleanup.dc.html): the cleanup
// level (features/ai.ts), the engines in the cleanup order (D36: this PC,
// your server, the cloud; features/ai.ts) with the "skipped at this level"
// warning, the model on this PC (features/models.ts), cloud cleanup on the
// Cloud provider (D78): OpenRouter's model, or the model on the user's
// LiteLLM proxy (D52), whose address is on the Cloud page
// (features/openrouter.ts), and an Advanced fold: one time limit for every
// engine, reading the screen, what else the cloud gets, the formatting
// rules before AI and the model file. The engine library is on Developer.
// Markup only.

import { ICON } from "../../lib/icons.js";
import { advanced, group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";
import { browseButton, LLM_DOWNLOAD_ROW } from "../../features/models.js";
import { engineRow } from "./speech.js";

/** A small switch after its label, for a stacked list of switches in one row. */
const small = (id: string, label: string): string =>
  `<label class="lw-small-switch">${label}${toggle(id, label).replace('class="fl-toggle"', 'class="fl-toggle is-small"')}</label>`;

export const settingsCleanupPage: Page = {
  id: "settings-cleanup",
  area: "settings",
  template: `
${heading(
  "AI cleanup",
  `Fixes punctuation, filler words and self-corrections. Tone per app is in <button type="button" class="lw-link" data-go="style">Style →</button>`,
)}
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
  <h3 class="fl-group-title">Engines</h3>
  <div class="fl-group" id="ai_routes">
    ${engineRow("aieng", "cleanup", "local", ICON.chip, "This PC", toggle("llm_enabled", "Use this PC for cleanup", `data-save="prefs"`), `data-needs="local"`)}
    ${engineRow("aieng", "cleanup", "server", ICON.server, "Your server", toggle("aieng_server_on", "Use your server for cleanup"), `data-needs="remote"`)}
    ${engineRow("aieng", "cleanup", "cloud", ICON.cloud, "Cloud", toggle("or_cleanup_allow", "Use the cloud for cleanup"), `data-needs="cloud"`)}
    <div class="fl-row lw-foot-row"><div class="fl-row-desc">The first ready engine is used; if it fails, the next one tries. <span id="or_cleanup_consent"></span> <span id="cleanup_order_msg" class="hint"></span></div></div>
  </div>
  <ul id="ai_warnings" class="lw-warnings" role="status"></ul>
</section>

${group("", LLM_DOWNLOAD_ROW, `data-needs="local"`)}

<section class="lw-section" id="or_cleanup_group" data-needs="cloud">
  <h3 class="fl-group-title lw-title-badge" id="or_cleanup_title"><span id="or_cleanup_title_name">Cloud · OpenRouter</span> <button type="button" class="lw-link lw-link-sm" data-go="settings-openrouter">Change provider →</button></h3>
  <div class="fl-group" id="or_cleanup_box">
    ${row(
      "Model",
      `From your own AI server's list, or type its name. <span id="cu_models_out"></span>`,
      `<input id="cu_model" class="fl-input lw-w220 lw-mono-input" list="cu_model_list" spellcheck="false" autocomplete="off" placeholder="gpt-4o-mini" aria-label="Model id" />
       <datalist id="cu_model_list"></datalist>
       <button type="button" class="fl-btn fl-btn-secondary" id="cu_models_load">Load models</button>`,
      `data-provider="custom"`,
    )}
    ${row(
      `<span id="or_cleanup_model_name"></span> <span id="or_cleanup_model_badges" class="pills"></span>`,
      `<span id="or_cleanup_model_desc"></span> <code id="or_cleanup_model_id" class="lw-mono"></code>`,
      `<button type="button" class="fl-btn fl-btn-secondary lw-btn-160" id="or_cleanup_change">Change</button>`,
      `data-provider="openrouter"`,
    )}
    ${row(
      "Prefer providers that are",
      "",
      `<select id="or_cleanup_sort" class="fl-select lw-w160">
        <option value="latency">Fastest</option>
        <option value="price">Cheapest</option>
        <option value="throughput">Highest throughput</option>
        <option value="">OpenRouter default</option>
      </select>`,
      `data-provider="openrouter"`,
    )}
    ${row(
      "Try it",
      `<span id="or_cleanup_test_out"></span> <span id="or_cleanup_state"></span> <span id="or_cleanup_msg"></span>`,
      `<button type="button" class="fl-btn fl-btn-secondary" id="or_cleanup_test">Test</button>`,
    )}
  </div>
</section>

${advanced(
  row(
    `<label for="llm_timeout_ms">Give up after</label>`,
    `Then the next engine tries.<span class="lw-status-sub" id="cleanup_timeout_note" hidden></span>`,
    `<select id="llm_timeout_ms" class="fl-select lw-w160" data-save="prefs">
      <option value="1500">1.5 seconds</option>
      <option value="3000">3 seconds</option>
      <option value="5000">5 seconds</option>
      <option value="8000">8 seconds</option>
      <option value="15000">15 seconds</option>
    </select>`,
  ) +
    row(
      "Use text on screen",
      "Reads the app you dictate into, on this PC only.",
      toggle("use_ax_context", "Use text on screen", `data-save="prefs"`),
    ) +
    row(
      "Also send to the cloud",
      "Needs Use text on screen.",
      `<div class="lw-switch-stack">
        ${small("or_cleanup_send_app_name", "App name")}
        ${small("or_cleanup_send_surrounding_text", "Text around the cursor")}
        ${small("or_cleanup_send_screen_words", "Words on screen")}
      </div>`,
      `data-needs="cloud"`,
    ) +
    row(
      "Formatting rules first",
      "Off: the AI gets your words as heard.",
      toggle("spoken_cleanup", "Formatting rules first", `data-save="prefs"`),
      `id="rules_row"`,
    ) +
    row(
      "Model file",
      "Empty = the default model.",
      `<input id="llm_model_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" aria-label="AI cleanup model file" />${browseButton("llm_model_path")}`,
      `data-needs="local"`,
    ),
  `id="cleanup_advanced"`,
)}`,
};
