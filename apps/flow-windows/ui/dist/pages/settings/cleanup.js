// Settings → AI cleanup: the cleanup level and formatting
// (features/ai.ts, features/general.ts), which engine cleans the text and
// OpenRouter cleanup (features/openrouter.ts). Markup only.
import { group, heading, row, toggle } from "../../lib/markup.js";
export const settingsCleanupPage = {
    id: "settings-cleanup",
    area: "settings",
    template: `
${heading("AI cleanup", `Turns what you said into what you meant: punctuation, self-corrections and tone. How it sounds in each app is set in <button type="button" class="lw-link" data-go="style">Style →</button>`)}
<section class="lw-section">
  <h3 class="fl-group-title">How much to change</h3>
  <div class="fl-group lw-pad">
    <div class="lw-seg" role="radiogroup" aria-label="Cleanup level" id="level_seg">
      <button type="button" role="radio" data-level="none">No AI</button>
      <button type="button" role="radio" data-level="verbatim">Verbatim</button>
      <button type="button" role="radio" data-level="light">Light</button>
      <button type="button" role="radio" data-level="medium">Medium</button>
      <button type="button" role="radio" data-level="high">High</button>
    </div>
    <select id="cleanup_level" data-save="prefs" hidden>
      <option value="none">No AI</option>
      <option value="verbatim">Verbatim</option>
      <option value="light">Light</option>
      <option value="medium">Medium</option>
      <option value="high">High</option>
    </select>
    <div class="lw-level-text">
      <div class="fl-row-title" id="level_title"></div>
      <div class="fl-row-desc" id="level_desc"></div>
    </div>
  </div>
</section>
${group("Formatting", row("Auto-format", `Punctuation, quotes, fillers and corrections: "comma", "new line", "quote … unquote", "um", "scratch that", "$5", "3pm". Runs on this PC with or without AI. Off = exactly what you said (same as Verbatim).`, toggle("spoken_cleanup", "Auto-format", `data-save="prefs"`)) +
        row("Use what's on screen", "Reads the text around the cursor to match names and context. Stays on this PC.", toggle("use_ax_context", "Use what's on screen", `data-save="prefs"`)))}
<section class="lw-section">
  <h3 class="fl-group-title">Engines</h3>
  <div class="fl-group">
    ${row("In use", `<span id="ai_engine_line">On this PC</span>`, "")}
    ${row("OpenRouter", `Sends the transcript as text to openrouter.ai. Asks first. <span id="or_cleanup_consent"></span>`, toggle("or_cleanup_allow", "Clean up text with OpenRouter"), `data-needs="cloud"`)}
    ${row("This PC", `<span id="llm_status">Off.</span>`, toggle("llm_enabled", "Clean up with a model on this PC", `data-save="prefs"`), `data-needs="local"`)}
    ${row("Order", "", `<select id="or_cleanup_order" class="fl-select is-wide">
        <option value="cloud,local">OpenRouter first, then this PC</option>
        <option value="local,cloud">This PC first, OpenRouter only when no model is loaded</option>
      </select>`, `data-needs="cloud"`)}
  </div>
</section>

<section class="lw-section" id="or_cleanup_group" data-needs="cloud">
  <h3 class="fl-group-title">OpenRouter</h3>
  <div class="fl-group">
    ${row(`<span id="or_cleanup_model_name"></span>`, `<code id="or_cleanup_model_id" class="lw-mono"></code> <span id="or_cleanup_model_badges" class="pills"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_cleanup_change">Change</button>`)}
    ${row("Prefer providers that are", "", `<select id="or_cleanup_sort" class="fl-select">
        <option value="latency">Fastest to answer</option>
        <option value="throughput">Highest throughput</option>
        <option value="price">Cheapest</option>
        <option value="">OpenRouter default</option>
      </select>`)}
    ${row("Give up after", "Milliseconds. Then this PC takes over.", `<input id="or_cleanup_timeout_ms" class="fl-input is-narrow" type="number" min="800" max="8000" step="100" placeholder="2500" />`)}
    ${row("Also send text around the cursor", "Your words, the app type, the writing style and your dictionary are always sent. More context gives better results.", toggle("or_cleanup_send_surrounding_text", "Send text around the cursor"))}
    ${row("Also send words on screen", "", toggle("or_cleanup_send_screen_words", "Send words on screen"))}
    ${row("Also send the app name", "", toggle("or_cleanup_send_app_name", "Send app name"))}
    ${row("Try it", `<span id="or_cleanup_test_out"></span> <span id="or_cleanup_state"></span> <span id="or_cleanup_msg"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_cleanup_test">Test</button>`)}
  </div>
  <p class="fl-help lw-group-help">Cloud cleanup is used for dictations on this PC and from OpenRouter. Your server uses its own AI.</p>
</section>

${group("On this PC", row("Model file", "A GGUF model. Qwen2.5-0.5B-Instruct Q4_K_M is the fast default; Qwen2.5-1.5B-Instruct is more accurate and about twice as slow. Never downloaded automatically. Empty = default.", `<input id="llm_model_path" class="fl-input is-wide" data-save="prefs" spellcheck="false" />`) +
        row("Engine library", `llama.cpp, <code id="llm_plugin" class="lw-mono"></code>. Empty = next to the app.`, `<input id="llm_lib_path" class="fl-input is-wide" data-save="prefs" spellcheck="false" />`) +
        row("Give up after", "Milliseconds per dictation (default 3000). Then the formatting rules take over.", `<input id="llm_timeout_ms" class="fl-input is-narrow" data-save="prefs" type="number" min="200" max="30000" step="100" />`), `data-needs="local"`)}`,
};
