// First-run setup markup (docs/design/app-design/Onboard*.dc.html): one
// static template per step. Static text only; user data goes in with
// textContent from onboarding/index.ts. Ids are prefixed `ob_`.
import { ICON } from "../lib/icons.js";
const CHECK = "M5 12.5l4.5 4.5L19 7";
const SHIELD = ICON.privacy;
const FILE = "M6 3h8l4 4v14H6zM14 3v4h4";
const FOLDER = "M3 6h6l2 2h10v11H3z";
export const STEPS = 5;
/** A stroke icon (`lw-ico` = 20 px inline icon). */
export function ico(path, cls = "lw-ico") {
    return `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"></path></svg>`;
}
const head = (step) => `
<div class="lw-ob-head">
  <span class="lw-ob-brand">Murmur</span>
  <span class="fl-overline">Step ${step} of ${STEPS}</span>
</div>`;
const foot = (back, next, hintId = "") => `
<div class="lw-ob-foot">
  ${back ? `<button type="button" class="fl-btn fl-btn-ghost" data-ob="back">Back</button>` : "<span></span>"}
  ${hintId ? `<span class="lw-ob-hint" id="${hintId}"></span>` : ""}
  <button type="button" class="fl-btn fl-btn-primary" data-ob="next">${next}</button>
</div>`;
const bullet = (b, rest) => `<li>${ico(CHECK, "lw-ico lw-ob-check")}<span><b>${b}</b> ${rest}</span></li>`;
const welcome = `
<div class="lw-ob-panel is-split" data-ob-step="welcome">
  <section class="lw-ob-intro">
    ${head(1)}
    <div class="lw-ob-intro-body">
      <h1 class="fl-serif lw-ob-h1 is-big">Speak, and it's <em>written</em>.</h1>
      <p class="lw-ob-lead">Hold a key, talk, let go. Murmur turns your voice into clean text in whatever app you're typing in.</p>
      <ul class="lw-ob-bullets">
        ${bullet("Runs on this PC.", "Works offline, no account.")}
        ${bullet("Nothing goes online", "unless you choose a server or OpenRouter.")}
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
const engineCard = (id, icon, title, desc, facts) => `
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
const engine = `
<div class="lw-ob-panel" data-ob-step="engine">
  ${head(2)}
  <div class="lw-ob-body is-center">
    <div class="lw-ob-title is-center">
      <h1 class="fl-serif lw-ob-h1">Where should your voice become text?</h1>
      <p class="lw-ob-lead">You can add the others later and use them as backups, in Settings → Speech to text.</p>
    </div>
    <div role="radiogroup" aria-label="Speech engine" class="lw-ob-cards">
      ${engineCard("pc", ICON.chip, "This PC", "Private and works offline. Fast on most computers from the last five years.", [
    "Needs a model file (0.15–0.7 GB)",
    "About 1 second after you stop talking",
    "Nothing leaves this PC",
])}
      ${engineCard("server", ICON.server, "Your server", "A faster computer you own does the work. Good for older laptops.", [
    "Needs flow-server running at home or on a VPN",
    "Audio goes only to your server",
    "This PC can step in if it’s offline",
])}
      ${engineCard("cloud", ICON.cloud, "OpenRouter", "A cloud model. No model file, but your audio leaves this computer.", [
    "Needs an OpenRouter API key",
    "Pay per use, a few cents an hour of audio",
    "Zero-retention providers only",
])}
    </div>
    <div class="fl-group lw-ob-detail" id="ob_server_detail" hidden>
      <div class="fl-row">
        <div class="fl-row-text"><label class="fl-row-title" for="ob_host">Server address</label><div class="fl-row-desc">A name or IP address, without ws:// or a port. <small class="field-err" id="ob_host_err"></small></div></div>
        <div class="fl-row-control"><input id="ob_host" class="fl-input is-wide" spellcheck="false" placeholder="100.64.0.10 or mybox.local" /></div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><label class="fl-row-title" for="ob_token">Access token</label><div class="fl-row-desc">From <code>flow-server gen-token</code>. Stored encrypted for your Windows account. <small class="field-err" id="ob_token_err"></small></div></div>
        <div class="fl-row-control"><input id="ob_token" class="fl-input" type="password" autocomplete="off" spellcheck="false" placeholder="from flow-server gen-token" /><button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="ob_test_conn">Test</button></div>
      </div>
      <div class="lw-ob-status" id="ob_server_status"></div>
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
  </div>
  ${foot(true, "Continue", "ob_engine_hint")}
</div>`;
const modelRow = (id, title, desc, input, placeholder) => `
<div class="fl-row lw-ob-model" data-ob-model="${id}">
  <span class="lw-tile">${ico(id === "parakeet" ? FOLDER : FILE, "")}</span>
  <div class="fl-row-text">
    <div class="fl-row-title">${title} <span class="fl-badge fl-badge-dark" data-ob-rec hidden>Recommended</span></div>
    <div class="fl-row-desc">${desc}</div>
  </div>
  <span class="fl-badge" data-ob-model-badge hidden></span>
  <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-ob-use="${id}">Use</button>
  <div class="lw-ob-model-path">
    <input id="${input}" class="fl-input" spellcheck="false" placeholder="${placeholder}" aria-label="${title} location" />
    <span class="lw-ob-model-status" data-ob-model-status></span>
  </div>
</div>`;
const model = `
<div class="lw-ob-panel" data-ob-step="model">
  ${head(3)}
  <div class="lw-ob-body is-split">
    <div class="lw-ob-side">
      <h1 class="fl-serif lw-ob-h1" id="ob_model_title">Add a speech model</h1>
      <p class="lw-ob-lead" id="ob_model_lead">The model is the part that listens. Get it once from where its makers publish it, then tell Murmur where it is.</p>
      <div class="lw-ob-note">
        ${ico(SHIELD, "lw-ico lw-ob-check")}
        <span>Murmur never downloads anything by itself. Your voice is turned into text by the model on this PC.</span>
      </div>
    </div>
    <div class="lw-ob-main">
      <div class="fl-group">
        ${modelRow("parakeet", "Parakeet TDT 0.6B", "English · fastest · a folder of about 640 MB", "ob_parakeet_dir", "Model folder")}
        ${modelRow("whisper", "Whisper base", "Many languages · one 148 MB file (ggml-base.bin)", "ob_whisper_path", "Model file")}
      </div>
      <div class="lw-ob-drop">
        ${ico("M12 15V4M8 8l4-4 4 4M5 13v7h14v-7", "lw-ico")}
        <div class="lw-ob-drop-text">
          <span>Put the files in <code class="lw-mono" id="ob_models_dir"></code>, or type where they are. Empty = the default name in that folder.</span>
          <span class="lw-ob-links">
            <span>Parakeet: <code class="lw-mono" id="ob_url_parakeet">github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models</code> <button type="button" class="lw-link lw-ob-copy" data-ob-copy="ob_url_parakeet">Copy</button></span>
            <span>Whisper: <code class="lw-mono" id="ob_url_whisper">huggingface.co/ggerganov/whisper.cpp</code> <button type="button" class="lw-link lw-ob-copy" data-ob-copy="ob_url_whisper">Copy</button></span>
          </span>
        </div>
      </div>
    </div>
  </div>
  ${foot(true, "Continue", "ob_model_hint")}
</div>`;
const cleanupOption = (id, title, desc) => `
<button type="button" role="radio" class="lw-ob-option" data-ob-cleanup="${id}" aria-checked="false">
  <span class="lw-ob-radio"></span>
  <span class="lw-ob-option-text">
    <span class="lw-ob-option-title">${title}<span class="fl-badge fl-badge-dark" data-ob-rec hidden>Recommended</span><span class="fl-badge fl-badge-light" data-ob-na hidden>Not in this build</span></span>
    <span class="lw-ob-option-desc">${desc}</span>
  </span>
</button>`;
const cleanup = `
<div class="lw-ob-panel" data-ob-step="cleanup">
  ${head(4)}
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
      ${cleanupOption("pc", "On this PC", "A small model on this PC. Private, adds about a second. Needs a 0.4–1 GB GGUF file.")}
      ${cleanupOption("cloud", "OpenRouter", "A large cloud model. Best quality, costs a few cents a month. Sends text, never audio.")}
      <div class="fl-group" id="ob_llm_detail" hidden>
        <div class="fl-row">
          <div class="fl-row-text"><label class="fl-row-title" for="ob_llm_path">Model file</label><div class="fl-row-desc" id="ob_llm_status"></div></div>
          <div class="fl-row-control"><input id="ob_llm_path" class="fl-input is-wide" spellcheck="false" /></div>
        </div>
        <p class="lw-ob-small">A GGUF chat model, e.g. Qwen2.5-1.5B-Instruct Q4_K_M from <code class="lw-mono" id="ob_url_llm">huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF</code> <button type="button" class="lw-link lw-ob-copy" data-ob-copy="ob_url_llm">Copy</button></p>
      </div>
      <div class="fl-group" id="ob_or_detail" hidden>
        <div class="fl-row" data-ob-key-row>
          <div class="fl-row-text"><label class="fl-row-title" for="ob_clean_key">OpenRouter API key</label><div class="fl-row-desc" data-ob-key-desc>Your words are sent as text, to zero-retention providers only.</div></div>
          <div class="fl-row-control"><input id="ob_clean_key" class="fl-input" data-ob-key type="password" autocomplete="off" spellcheck="false" placeholder="sk-or-…" /></div>
        </div>
        <label class="fl-row lw-ob-consent">
          <input type="checkbox" id="ob_clean_consent" />
          <span class="fl-row-text"><span class="fl-row-title">Send my dictated text to OpenRouter</span><span class="fl-row-desc">After each dictation the text and your dictionary words go to openrouter.ai and the provider it routes to. Your voice is not sent unless OpenRouter speech to text is also on.</span></span>
        </label>
      </div>
      <div class="lw-ob-status" id="ob_cleanup_status"></div>
    </div>
  </div>
  ${foot(true, "Continue")}
</div>`;
const tryIt = `
<div class="lw-ob-panel" data-ob-step="try">
  ${head(5)}
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
          <input id="ob_ptt" class="fl-input" spellcheck="false" placeholder="Ctrl+Win" aria-label="Push to talk shortcut" hidden />
          <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="ob_ptt_change">Change</button>
        </div>
      </div>
    </div>
  </div>
  ${foot(true, "Finish")}
</div>`;
export const TEMPLATE = `
<div class="lw-ob" id="onboarding" role="dialog" aria-modal="true" aria-label="Set up Murmur" hidden>
  ${welcome}${engine}${model}${cleanup}${tryIt}
</div>`;
