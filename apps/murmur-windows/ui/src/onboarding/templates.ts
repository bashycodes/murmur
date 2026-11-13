// First-run setup markup (docs/design/app-design/Onboard*.dc.html): one
// static template per step. Static text only; user data goes in with
// textContent from onboarding/index.ts. Ids are prefixed `ob_`.

import { extLink, SERVER_HELP } from "../features/models.js";
import { ICON } from "../lib/icons.js";

const CHECK = "M5 12.5l4.5 4.5L19 7";
const SHIELD = ICON.privacy;
const FILE = "M6 3h8l4 4v14H6zM14 3v4h4";

export const STEPS = 5;

/** The cleanup model's page, for adding the file by hand (opens in the browser). */
export const URL_LLM = "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF";

/** A stroke icon (`lw-ico` = 20 px inline icon). */
export function ico(path: string, cls = "lw-ico"): string {
  return `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"></path></svg>`;
}

const head = (step: number): string => `
<div class="lw-ob-head">
  <span class="lw-ob-brand">Murmur</span>
  <span class="fl-overline">Step ${step} of ${STEPS}</span>
</div>`;

/** The footer bar (own background, never scrolls): Back, a hint, Continue, lined up with the content. */
const foot = (back: boolean, next: string, hintId = "", extra = ""): string => `
<div class="lw-ob-foot">
  <div class="lw-ob-frame lw-ob-foot-row">
    ${back ? `<button type="button" class="fl-btn fl-btn-ghost" data-ob="back">Back</button>` : "<span></span>"}
    ${hintId ? `<span class="lw-ob-hint" id="${hintId}"></span>` : ""}
    ${extra}
    <button type="button" class="fl-btn fl-btn-primary" data-ob="next">${next}</button>
  </div>
</div>`;

/**
 * Steps 2-5 share one layout: the head and the step's content scroll, top
 * aligned, above the footer bar, all in one centred column, so a pick that
 * shows or hides fields never moves what is above it.
 */
const stepPanel = (name: string, n: number, body: string, footer: string): string => `
<div class="lw-ob-panel is-framed" data-ob-step="${name}">
  <div class="lw-ob-scroll" data-ob-scroll>
    <div class="lw-ob-frame">
      ${head(n)}
      ${body}
    </div>
  </div>
  ${footer}
</div>`;

const bullet = (b: string, rest: string): string =>
  `<li>${ico(CHECK, "lw-ico lw-ob-check")}<span><b>${b}</b> ${rest}</span></li>`;

const welcome = `
<div class="lw-ob-panel is-split" data-ob-step="welcome">
  <section class="lw-ob-intro">
    ${head(1)}
    <div class="lw-ob-intro-body">
      <h1 class="fl-serif lw-ob-h1 is-big">Speak, and it's <em>written</em>.</h1>
      <p class="lw-ob-lead">Hold a key, talk, let go. Murmur turns your voice into clean text in whatever app you're typing in.</p>
      <ul class="lw-ob-bullets">
        ${bullet("Runs on this PC.", "Works offline, no account.")}
        ${bullet("Nothing goes online", "unless you choose to.")}
        ${bullet("About 3 minutes", "to set up.")}
      </ul>
      <div class="lw-ob-actions">
        <button type="button" class="fl-btn fl-btn-primary lw-ob-lg" data-ob="next">Get started</button>
        <button type="button" class="fl-btn fl-btn-ghost" data-ob="skip">Set up later</button>
      </div>
    </div>
  </section>
  <section class="lw-ob-art" aria-hidden="true">
    <div class="lw-ob-mail">
      <div class="lw-ob-mail-bar"><span class="lw-dot"></span><span class="lw-dot"></span><span class="lw-dot"></span><span>New message</span></div>
      <div class="lw-ob-mail-rule"></div>
      <div class="lw-ob-mail-text">Hi Sam,<br /><br />Thursday at 3 works. I'll bring the slides and the latest numbers.<span class="lw-ob-caret"></span></div>
    </div>
    <div class="lw-ob-pill">${[8, 14, 20, 12, 18, 9, 15, 6].map((h) => `<span style="height:${h}px"></span>`).join("")}</div>
  </section>
</div>`;

const engineCard = (id: string, icon: string, title: string, desc: string, facts: string[]): string => `
<button type="button" role="radio" class="lw-ob-card" data-ob-engine="${id}" aria-checked="false">
  <span class="lw-ob-card-top">
    <span class="lw-tile">${ico(icon, "")}</span>
    <span class="fl-badge fl-badge-dark" data-ob-rec hidden>Recommended</span>
    <span class="fl-badge fl-badge-light" data-ob-na hidden>Not in this build</span>
  </span>
  <span class="lw-ob-card-title">${title}</span>
  <span class="lw-ob-card-desc">${desc}</span>
  <span class="lw-ob-facts">${facts.map((f) => `<span>${f}</span>`).join("")}</span>
</button>`;

const engine = stepPanel(
  "engine",
  2,
  `
  <div class="lw-ob-body is-center">
    <div class="lw-ob-title is-center">
      <h1 class="fl-serif lw-ob-h1">Where should your voice become text?</h1>
      <p class="lw-ob-lead">You can add the others later and use them as backups, in Settings → Speech to text.</p>
    </div>
    <div role="radiogroup" aria-label="Speech engine" class="lw-ob-cards">
      ${engineCard("pc", ICON.chip, "This PC", "Private, works offline.", [
        "One download of 0.15–0.7 GB",
        "About 1 second after you stop talking",
        "Nothing leaves this PC",
      ])}
      ${engineCard("server", ICON.server, "Your server", "Use a faster computer you own.", [
        "Good for older laptops",
        "Audio goes only to your computer",
        "This PC can step in if it’s off",
      ])}
      ${engineCard("cloud", ICON.cloud, "Cloud", "Best accuracy. Audio is sent online.", [
        "Needs an account key",
        "Pay per use, a few cents an hour of audio",
        "Nothing to download",
      ])}
    </div>
    <div class="lw-ob-server-slot" data-ob-server-slot="engine">
    <div class="lw-ob-detail lw-ob-server" id="ob_server_detail" hidden>
      <div class="lw-ob-server-bar">
        <button type="button" class="lw-link lw-ob-copy lw-ob-more" id="ob_server_more" aria-expanded="false" aria-controls="ob_server_fields">I already have a server <span class="lw-ob-chev" aria-hidden="true">▸</span></button>
        <span class="lw-ob-small"><span id="ob_server_help">No server yet? Pick This PC above, or</span> ${extLink(SERVER_HELP, "set up a server →", "lw-link lw-ob-copy")}</span>
      </div>
      <div class="fl-group" id="ob_server_fields" hidden>
        <div class="fl-row">
          <div class="fl-row-text"><label class="fl-row-title" for="ob_host">Server address</label><div class="fl-row-desc">Its name or IP address, e.g. mybox.local <small class="field-err" id="ob_host_err"></small></div></div>
          <div class="fl-row-control"><input id="ob_host" class="fl-input is-wide" spellcheck="false" placeholder="mybox.local" /></div>
        </div>
        <div class="fl-row">
          <div class="fl-row-text"><label class="fl-row-title" for="ob_token">Access token</label><div class="fl-row-desc"><span id="ob_token_desc">Shown when you set up your server. Stored encrypted for your Windows account.</span> <small class="field-err" id="ob_token_err"></small></div></div>
          <div class="fl-row-control"><input id="ob_token" class="fl-input is-wide" type="password" autocomplete="off" spellcheck="false" placeholder="Paste the token" /><button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="ob_test_conn">Test</button></div>
        </div>
        <div class="lw-ob-status" id="ob_server_status"></div>
      </div>
    </div>
    </div>
    <div class="fl-group lw-ob-detail" id="ob_cloud_detail" hidden>
      <div class="fl-row" data-ob-key-row>
        <div class="fl-row-text"><label class="fl-row-title" for="ob_stt_key">OpenRouter API key</label><div class="fl-row-desc" data-ob-key-desc>Kept in Windows Credential Manager, never shown again.</div></div>
        <div class="fl-row-control"><input id="ob_stt_key" class="fl-input" data-ob-key type="password" autocomplete="off" spellcheck="false" placeholder="sk-or-…" /></div>
      </div>
      <label class="fl-row lw-ob-consent">
        <input type="checkbox" id="ob_stt_consent" />
        <span class="fl-row-text"><span class="fl-row-title">Send my audio to OpenRouter</span><span class="fl-row-desc">The audio of each dictation and your dictionary words go to openrouter.ai and the provider it routes to (zero-retention only). You can turn this off at any time.</span></span>
      </label>
      <div class="lw-ob-status" id="ob_cloud_status"></div>
    </div>
  </div>`,
  foot(true, "Continue", "ob_engine_hint"),
);

/**
 * "Download from: Your server | Hugging Face" (D79) on a step, and the slot the
 * step-2 server fields move into when "Your server" is picked with none saved.
 */
const source = (step: string): string => `
<div class="lw-ob-src" data-ob-src-box hidden>
  <span class="fl-row-title" id="ob_src_label_${step}">Download from</span>
  <div class="lw-seg" role="radiogroup" aria-labelledby="ob_src_label_${step}">
    <button type="button" role="radio" aria-checked="false" data-ob-src="server">Your server</button>
    <button type="button" role="radio" aria-checked="false" data-ob-src="huggingface">Hugging Face</button>
  </div>
  <span class="lw-ob-src-hint" data-ob-src-hint></span>
</div>
<div class="lw-ob-server-slot" data-ob-server-slot="${step}"></div>`;

/**
 * A speech model row (step 3): picked like step 4's options, with its state
 * chip (Installed / Downloading 34% / Not installed), Download / Cancel and
 * the download's progress (3b). A div, not a button: it holds buttons.
 */
const speechRow = (id: string, title: string, desc: string): string => `
<div class="lw-ob-option lw-ob-speech" role="radio" aria-checked="false" tabindex="0" data-ob-model="${id}">
  <span class="lw-ob-radio"></span>
  <span class="lw-ob-option-text">
    <span class="lw-ob-option-title"><span data-ob-model-title>${title}</span><span class="fl-badge fl-badge-dark" data-ob-rec hidden>Recommended</span></span>
    <span class="lw-ob-option-desc lw-ob-speech-state"><span class="fl-badge" data-ob-chip></span><span data-ob-model-desc>${desc}</span></span>
  </span>
  <span class="lw-ob-model-actions">
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-ob-dl="${id}" hidden>Download</button>
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-ob-dl-cancel="${id}" hidden>Cancel</button>
  </span>
  <div class="lw-ob-dl" data-ob-dl-box hidden>
    <div class="lw-ob-bar" role="progressbar" aria-label="Downloading ${title}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" data-ob-dl-bar><span></span></div>
    <span class="lw-ob-dl-line" data-ob-dl-line></span>
  </div>
</div>`;

const model = stepPanel(
  "model",
  3,
  `
  <div class="lw-ob-body is-split">
    <div class="lw-ob-side">
      <h1 class="fl-serif lw-ob-h1">Add a speech model</h1>
      <p class="lw-ob-lead">The model is the part that listens. It runs on this PC, so your voice stays here. One download, once.</p>
      <div class="lw-ob-note">
        ${ico(SHIELD, "lw-ico lw-ob-check")}
        <span id="ob_model_note">A download only fetches the model. Nothing about you or your voice is sent.</span>
      </div>
    </div>
    <div role="radiogroup" aria-label="Speech model" class="lw-ob-main">
      ${speechRow("parakeet", "English — fast", "")}
      <button type="button" class="lw-link lw-ob-copy lw-ob-more" id="ob_model_more" aria-expanded="false" aria-controls="ob_model_other">More languages <span class="lw-ob-chev" aria-hidden="true">▸</span></button>
      <div id="ob_model_other" hidden>
        ${speechRow("whisper", "Whisper base", "Many languages")}
      </div>
      <div class="lw-ob-status" id="ob_model_status"></div>
      <p class="lw-ob-small">Have a model file already? Choose it after setup in Settings → Speech to text.</p>
    </div>
  </div>`,
  foot(true, "Continue", "ob_model_hint"),
);

const cleanupOption = (id: string, title: string, desc: string): string => `
<button type="button" role="radio" class="lw-ob-option" data-ob-cleanup="${id}" aria-checked="false">
  <span class="lw-ob-radio"></span>
  <span class="lw-ob-option-text">
    <span class="lw-ob-option-title">${title}<span class="fl-badge fl-badge-dark" data-ob-rec hidden>Recommended</span><span class="fl-badge fl-badge-light" data-ob-na hidden>Not in this build</span></span>
    <span class="lw-ob-option-desc" data-ob-desc>${desc}</span>
    <span class="lw-ob-option-desc" data-ob-reason hidden></span>
  </span>
</button>`;

const cleanup = stepPanel(
  "cleanup",
  4,
  `
  <div class="lw-ob-body is-split">
    <div class="lw-ob-side">
      <h1 class="fl-serif lw-ob-h1">Polish what you say?</h1>
      <p class="lw-ob-lead">AI cleanup fixes run-on sentences, drops "um"s, applies your corrections and matches the tone of each app.</p>
      <div class="lw-ob-sample">
        <span class="fl-overline">You said</span>
        <span class="lw-ob-said">um so lunch is tuesday no wait thursday</span>
        <span class="fl-overline">Pasted</span>
        <span class="lw-ob-pasted" id="ob_sample"></span>
      </div>
    </div>
    <div role="radiogroup" aria-label="AI cleanup" class="lw-ob-main">
      ${cleanupOption("rules", "Rules only", 'Instant. Punctuation, numbers and "scratch that" by fixed rules. No model needed.')}
      ${cleanupOption("pc", "On this PC", "A small AI model on this PC. Private, adds about a second. One download of about 1 GB.")}
      ${cleanupOption("server", "Your server", "Your server's AI model cleans the text. Nothing to download here, and the text goes only to your server.")}
      ${cleanupOption("cloud", "Cloud", "A large online AI model. Best quality. Sends text, never audio.")}
      <div class="lw-ob-llm" id="ob_llm_detail" hidden>
        ${source("cleanup")}
        <div class="fl-group">
          <div class="fl-row lw-ob-model is-on" id="ob_llm_dl" hidden>
            <span class="lw-tile">${ico(FILE, "")}</span>
            <div class="fl-row-text">
              <div class="fl-row-title" id="ob_llm_dl_title"></div>
              <div class="fl-row-desc" id="ob_llm_dl_desc"></div>
            </div>
            <span class="lw-ob-model-actions">
              <span class="fl-badge fl-badge-light" id="ob_llm_missing" hidden>Not on your server</span>
              <button type="button" class="fl-btn fl-btn-primary fl-btn-sm" id="ob_llm_dl_start" hidden>Download</button>
              <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="ob_llm_dl_cancel" hidden>Cancel</button>
            </span>
            <div class="lw-ob-dl" id="ob_llm_dl_box" hidden>
              <div class="lw-ob-bar" role="progressbar" aria-label="Downloading the cleanup model" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" id="ob_llm_dl_bar"><span></span></div>
              <span class="lw-ob-dl-line" id="ob_llm_dl_line"></span>
            </div>
          </div>
          <div class="fl-row">
            <div class="fl-row-text"><label class="fl-row-title" for="ob_llm_path">Model file</label><div class="fl-row-desc" id="ob_llm_status"></div></div>
            <div class="fl-row-control"><input id="ob_llm_path" class="fl-input is-wide" spellcheck="false" /><button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" data-ob-pick="llm">Choose file</button></div>
          </div>
        </div>
        <p class="lw-ob-small">Or choose a model file you already have.<span id="ob_llm_hf"> ${extLink(URL_LLM, "Get it from Hugging Face →", "lw-link lw-ob-copy")}</span></p>
      </div>
      <div class="fl-group" id="ob_or_detail" hidden>
        <label class="fl-row lw-ob-consent">
          <input type="checkbox" id="ob_clean_consent" />
          <span class="fl-row-text"><span class="fl-row-title" id="ob_clean_consent_title">Send my dictated text to OpenRouter</span><span class="fl-row-desc" id="ob_clean_consent_desc"></span></span>
        </label>
        <p class="lw-ob-small lw-ob-later">Add details later in Settings → AI provider. <span class="lw-ob-missing" id="ob_cloud_missing" hidden></span></p>
      </div>
      <div class="lw-ob-status" id="ob_cleanup_status"></div>
    </div>
  </div>`,
  foot(true, "Continue"),
);

const tryIt = stepPanel(
  "try",
  5,
  `
  <div class="lw-ob-body is-center is-tight">
    <div class="lw-ob-title is-center">
      <h1 class="fl-serif lw-ob-h1">Now try it</h1>
      <div class="fl-kbd-group"><span id="ob_keys"></span><span class="fl-kbd-text">Hold, say anything, then let go</span></div>
    </div>
    <div class="lw-ob-try">
      <label for="ob_try_box" class="fl-overline">Practice box</label>
      <textarea id="ob_try_box" class="lw-ob-try-box" rows="4" placeholder="Click here, hold the shortcut and say something."></textarea>
      <div class="lw-ob-result" id="ob_result" hidden>
        ${ico(CHECK, "lw-ico")}
        <span id="ob_result_text"></span>
      </div>
    </div>
    <div class="fl-group lw-ob-try">
      <div class="fl-row">
        <div class="fl-row-text"><label class="fl-row-title" for="ob_mic">Microphone</label></div>
        <div class="fl-row-control"><select id="ob_mic" class="fl-select is-wide"></select></div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Shortcut</div><div class="fl-row-desc" id="ob_shortcut_desc"></div></div>
        <div class="fl-row-control">
          <span id="ob_ptt_rec" class="lw-ob-rec" role="status" hidden>Press keys… Esc to cancel</span>
          <button type="button" class="lw-link lw-ob-type" id="ob_ptt_type" hidden>Type instead</button>
          <input id="ob_ptt" class="fl-input" spellcheck="false" placeholder="Right Alt" aria-label="Push to talk shortcut" hidden />
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="ob_ptt_change">Change</button>
        </div>
      </div>
    </div>
  </div>`,
  foot(
    true,
    "Finish",
    "",
    `<span class="lw-ob-summary" id="ob_summary" hidden>
      <span id="ob_summary_text"></span>
      <button type="button" class="lw-link lw-ob-copy" id="ob_summary_keep" title="Put back the settings you had before this run">Keep my old setup</button>
      <button type="button" class="lw-link lw-ob-copy" id="ob_summary_change">Change</button>
    </span>`,
  ),
);

export const TEMPLATE = `
<div class="lw-ob" id="onboarding" role="dialog" aria-modal="true" aria-label="Set up Murmur" hidden>
  ${welcome}${engine}${model}${cleanup}${tryIt}
</div>`;
