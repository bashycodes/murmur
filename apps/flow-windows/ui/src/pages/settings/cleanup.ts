// Settings → AI cleanup (design: SettingsCleanup.dc.html): the cleanup
// level and formatting (features/ai.ts, features/general.ts), which engine
// cleans the text and in what order, and OpenRouter cleanup
// (features/openrouter.ts). Markup only.

import { ICON } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";
import { engineRow } from "./speech.js";

/** A small switch with its label, for a stacked list of switches in one row. */
const small = (id: string, label: string): string =>
  `<label class="lw-small-switch">${toggle(id, label).replace('class="fl-toggle"', 'class="fl-toggle is-small"')}${label}</label>`;

export const settingsCleanupPage: Page = {
  id: "settings-cleanup",
  area: "settings",
  template: `
${heading(
  "AI cleanup",
  `Turns what you said into what you meant: punctuation, paragraphs, lists, self-corrections and tone. How it sounds in each app is set in <button type="button" class="lw-link" data-go="style">Style →</button>`,
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

<section class="lw-section">
  <h3 class="fl-group-title">Engines</h3>
  <div class="fl-group" id="ai_routes">
    ${engineRow("aieng", "cleanup", "cloud", ICON.cloud, "OpenRouter", toggle("or_cleanup_allow", "Use OpenRouter for cleanup"), `data-needs="cloud"`)}
    ${engineRow("aieng", "cleanup", "local", ICON.chip, "This PC", toggle("llm_enabled", "Use this PC for cleanup", `data-save="prefs"`), `data-needs="local"`)}
    <div class="fl-row lw-foot-row"><div class="fl-row-desc">If both fail or run out of time, the built-in formatting rules finish the job. Dictations sent to your server are cleaned up there. Now: <span id="ai_engine_line">On this PC</span>. <span id="or_cleanup_consent"></span></div></div>
  </div>
  <select id="or_cleanup_order" hidden aria-hidden="true" tabindex="-1">
    <option value="cloud,local">OpenRouter first, then this PC</option>
    <option value="local,cloud">This PC first, OpenRouter only when no model is loaded</option>
  </select>
</section>

<section class="lw-section" id="or_cleanup_group" data-needs="cloud">
  <h3 class="fl-group-title">OpenRouter</h3>
  <div class="fl-group" id="or_cleanup_box">
    ${row(
      `<span id="or_cleanup_model_name"></span> <span id="or_cleanup_model_badges" class="pills"></span>`,
      `<span id="or_cleanup_model_desc"></span> <code id="or_cleanup_model_id" class="lw-mono"></code>`,
      `<button type="button" class="fl-btn fl-btn-secondary lw-btn-160" id="or_cleanup_change">Change</button>`,
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
    )}
    ${row(
      "Give up after",
      "Then this PC takes over.",
      `<select id="or_cleanup_timeout_ms" class="fl-select lw-w160">
        <option value="1500">1.5 seconds</option>
        <option value="2500">2.5 seconds</option>
        <option value="5000">5 seconds</option>
        <option value="8000">8 seconds</option>
      </select>`,
    )}
    ${row(
      "Also send",
      "Your words, the app type, the writing style and your dictionary are always sent. More context gives better results.",
      `<div class="lw-switch-stack">
        ${small("or_cleanup_send_surrounding_text", "Text around the cursor")}
        ${small("or_cleanup_send_screen_words", "Words on screen")}
        ${small("or_cleanup_send_app_name", "App name")}
      </div>`,
    )}
    ${row(
      "Try it",
      `<span id="or_cleanup_test_out"></span> <span id="or_cleanup_state"></span> <span id="or_cleanup_msg"></span>`,
      `<button type="button" class="fl-btn fl-btn-secondary" id="or_cleanup_test">Test</button>`,
    )}
  </div>
</section>

${group(
  "On this PC",
  row(
    "Model file",
    "A GGUF model. Qwen2.5-0.5B-Instruct Q4_K_M is the fast default; Qwen2.5-1.5B-Instruct is more accurate and about twice as slow. Never downloaded automatically. Empty = default.",
    `<input id="llm_model_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />`,
    `data-needs="local"`,
  ) +
    row(
      "Engine library",
      `llama.cpp, <code id="llm_plugin" class="lw-mono"></code>. Empty = next to the app.`,
      `<input id="llm_lib_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />`,
      `data-needs="local"`,
    ) +
    row(
      "Give up after",
      "Then the formatting rules take over.",
      `<select id="llm_timeout_ms" class="fl-select lw-w160" data-save="prefs">
        <option value="1500">1.5 seconds</option>
        <option value="3000">3 seconds</option>
        <option value="6000">6 seconds</option>
        <option value="10000">10 seconds</option>
      </select>`,
      `data-needs="local"`,
    ) +
    row(
      "Use what's on screen",
      "Reads nearby text in the app to match names and context. Stays on this PC.",
      toggle("use_ax_context", "Use what's on screen", `data-save="prefs"`),
    ) +
    row(
      "Formatting rules",
      `Punctuation, quotes, fillers and corrections: "comma", "new line", "quote … unquote", "um", "scratch that", "$5", "3pm". Run on this PC with or without AI. Off = exactly what you said (same as Verbatim).`,
      toggle("spoken_cleanup", "Formatting rules", `data-save="prefs"`),
    ),
)}`,
};
