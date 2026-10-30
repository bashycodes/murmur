// Settings → Speech to text: the engine order (features/engines.ts), the
// model on this PC (features/general.ts) and OpenRouter speech-to-text
// (features/openrouter.ts). This file is markup only.
import { ICON, iconSvg } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
/** One engine in the order list; features/engines.ts moves and fills it. */
function engineRow(route, icon, title, control) {
    return `
<div class="fl-row lw-engine" data-route="${route}" id="eng_row_${route}">
  <span class="lw-rank"></span>
  <span class="lw-tile">${iconSvg(icon)}</span>
  <div class="fl-row-text"><div class="fl-row-title">${title}</div><div class="fl-row-desc" id="eng_desc_${route}"></div></div>
  <span class="fl-badge fl-badge-light" id="eng_badge_${route}"></span>
  <span class="lw-move">
    <button type="button" class="fl-iconbtn is-toolbar" data-move="up" aria-label="Move ${title} up">${iconSvg(ICON.up, 16)}</button>
    <button type="button" class="fl-iconbtn is-toolbar" data-move="down" aria-label="Move ${title} down">${iconSvg(ICON.down, 16)}</button>
  </span>
  ${control}
</div>`;
}
export const settingsSpeechPage = {
    id: "settings-speech",
    area: "settings",
    template: `
${heading("Speech to text", "Turns your voice into words. Engines are tried top to bottom; if one fails, the next one takes over.")}
<section class="lw-section" id="eng_group">
  <h3 class="fl-group-title">Engines</h3>
  <div class="fl-group" id="dev_routes">
    ${engineRow("server", ICON.server, "Your server", toggle("dev_allow_server", "Use your server", `data-save="dev"`))}
    ${engineRow("client", ICON.chip, "This PC", toggle("dev_allow_client", "Use this PC", `data-save="dev"`))}
    ${engineRow("cloud", ICON.cloud, "OpenRouter", toggle("or_stt_allow", "Use OpenRouter for speech"))}
  </div>
  <ul id="dev_warnings" class="lw-warnings"></ul>
</section>

${group("On this PC", row("Model", `<span id="model_status">Checking…</span>`, `<select id="asr_engine" class="fl-select is-wide" data-save="prefs">
      <option value="whisper">Whisper (any language)</option>
      <option value="parakeet">Parakeet (English only, faster)</option>
    </select>`) +
        row("Model file", "A whisper.cpp ggml model, e.g. <code>ggml-base.bin</code> (default) or <code>ggml-small-q5_1.bin</code> for better accuracy. Empty = default.", `<input id="model_path" class="fl-input is-wide" data-save="prefs" spellcheck="false" />`, `data-engine="whisper"`) +
        row("Model folder", "The <code>sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8</code> folder. Empty = default.", `<input id="parakeet_model_dir" class="fl-input is-wide" data-save="prefs" spellcheck="false" />`, `data-engine="parakeet"`) +
        row("Use dictionary words when listening", "Parakeet only. Better at names and jargon; greedy decoding uses about 0.6 GB less memory. Changing it reloads the model.", `<select id="dev_parakeet_decoding" class="fl-select" data-save="dev">
        <option value="auto">Yes (beam search)</option>
        <option value="greedy">No (greedy)</option>
      </select>`, `data-needs="dev"`) +
        row("Keep loaded while the server is used", "Instant fallback if the server drops, at the cost of memory.", toggle("dev_keep_local_warm", "Keep loaded while the server is used", `data-save="dev"`), `data-needs="remote"`), `data-needs="local"`)}
<p class="fl-help lw-group-help" data-needs="local">Models are never downloaded automatically: put them in <code id="models_dir" class="lw-mono"></code>.</p>

${group("", row("Transcribe while I speak", "Each phrase is turned into text as you pause, so a long dictation is ready about as fast as a short one. Off = the whole recording is transcribed after you stop. OpenRouter always gets the whole recording.", toggle("dev_decode_while_speaking", "Transcribe while I speak", `data-save="dev"`)), `data-needs="dev"`)}

<section class="lw-section" id="or_stt_group" data-needs="cloud">
  <h3 class="fl-group-title">OpenRouter</h3>
  <div class="fl-group">
    ${row(`<span id="or_stt_model_name"></span>`, `<code id="or_stt_model_id" class="lw-mono"></code> <span id="or_stt_model_badges" class="pills"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_stt_change">Change</button>`)}
    ${row("Language", "Empty = detect automatically.", `<input id="or_stt_language" class="fl-input" spellcheck="false" placeholder="auto" />`)}
    ${row("Speech path", "A chat model with audio input may paraphrase instead of writing down what you said.", `<select id="or_stt_path" class="fl-select is-wide">
        <option value="auto">Auto (from the catalogue)</option>
        <option value="transcriptions">Transcription endpoint</option>
        <option value="chat_audio">Chat model with audio input</option>
      </select>`)}
    ${row("Send dictionary words", "Helps it spell names the way you do.", toggle("or_stt_send_dictionary", "Send dictionary words"))}
    ${row("Try it", `<span id="or_stt_test_out"></span> <span id="or_stt_consent"></span> <span id="or_stt_status"></span>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="or_stt_test">Test</button>`)}
  </div>
</section>`,
};
