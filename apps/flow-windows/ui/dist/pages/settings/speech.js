// Settings → Speech to text (design: SettingsSpeech.dc.html): the engine
// order and the meeting order (D47) (features/engines.ts), the model on
// this PC (features/general.ts) and OpenRouter speech to text
// (features/openrouter.ts). Markup only.
import { ICON, iconSvg } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import { NOTETAKER_ENABLED } from "../notetaker.js";
import { browseButton, DOWNLOAD_ROW, HEBREW_ROW, SERVER_MODELS_GROUP, SOURCE_ROW } from "../../features/models.js";
/** One engine in a ranked list (`<prefix>_row_<key>`, `_desc_`, `_badge_`):
 *  rank, tile, text, status badge, move up/down, switch. The feature module
 *  moves the row, numbers it and fills the text and badge. `keyAttr` names
 *  the data attribute that holds `key` (e.g. `route`). */
export function engineRow(prefix, keyAttr, key, icon, title, control, attrs = "") {
    return `
<div class="fl-row lw-engine" data-${keyAttr}="${key}" id="${prefix}_row_${key}"${attrs ? ` ${attrs}` : ""}>
  <span class="lw-rank"></span>
  <span class="lw-tile">${iconSvg(icon)}</span>
  <div class="fl-row-text"><div class="fl-row-title">${title}</div><div class="fl-row-desc" id="${prefix}_desc_${key}"></div></div>
  <span class="fl-badge fl-badge-light" id="${prefix}_badge_${key}"></span>
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
${heading("Speech to text", "Turns your voice into words. Engines are tried top to bottom; if one fails, the next one picks up the same audio and nothing you said is lost.")}
<section class="lw-section" id="eng_group">
  <h3 class="fl-group-title">Engines</h3>
  <div class="fl-group" id="dev_routes">
    ${engineRow("eng", "route", "client", ICON.chip, "This PC", toggle("dev_allow_client", "Use this PC", `data-save="dev"`))}
    ${engineRow("eng", "route", "server", ICON.server, "Your server", toggle("dev_allow_server", "Use your server", `data-save="dev"`))}
    ${engineRow("eng", "route", "cloud", ICON.cloud, "Cloud", toggle("or_stt_allow", "Use the cloud for speech"))}
  </div>
  <ul id="dev_warnings" class="lw-warnings"></ul>
</section>

<section class="lw-section" id="meet_group"${NOTETAKER_ENABLED ? "" : " hidden"}>
  <h3 class="fl-group-title">Meetings</h3>
  <div class="fl-group" id="meet_routes">
    ${engineRow("meet", "meeting", "client", ICON.chip, "This PC", toggle("meet_on_client", "Transcribe meetings on this PC"))}
    ${engineRow("meet", "meeting", "server", ICON.server, "Your server", toggle("meet_on_server", "Transcribe meetings on your server"))}
    ${engineRow("meet", "meeting", "cloud", ICON.cloud, "Cloud", toggle("meet_on_cloud", "Transcribe meetings in the cloud"))}
    <div class="fl-row lw-foot-row"><div class="fl-row-desc">Recorded meetings are transcribed after they end by the first engine that is on and answers; if it fails, the next one picks up the same part of the meeting. Dictation always goes first. <span id="meet_follow"></span> <span id="meet_order_msg" class="hint"></span></div></div>
  </div>
</section>

${group("On this PC", row("Model", `Parakeet is fastest but English only. Whisper speaks many languages.<span class="lw-status-sub"><span id="model_status">Checking…</span></span>`, `<select id="asr_engine" class="fl-select" data-save="prefs">
      <option value="parakeet">Parakeet (English)</option>
      <option value="whisper">Whisper (multilingual)</option>
    </select>`) +
        row("Model file", `A whisper.cpp ggml model, e.g. <code>ggml-base.bin</code> (default) or <code>ggml-small-q5_1.bin</code> for better accuracy. Empty = default.`, `<input id="model_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />${browseButton("model_path")}`, `data-engine="whisper"`) +
        row("Model folder", "The <code>sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8</code> folder. Empty = default.", `<input id="parakeet_model_dir" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" />${browseButton("parakeet_model_dir")}`, `data-engine="parakeet"`) +
        SOURCE_ROW +
        DOWNLOAD_ROW +
        HEBREW_ROW +
        row("Use dictionary words when listening", "Better at names and jargon. Uses about 0.6 GB more memory; changing it reloads the model.", toggle("dev_parakeet_decoding", "Use dictionary words when listening"), `data-needs="dev" id="dict_words_row"`) +
        row("Keep loaded while the server is used", "Instant fallback if the server drops, at the cost of memory.", toggle("dev_keep_local_warm", "Keep loaded while the server is used", `data-save="dev"`), `data-needs="remote"`), `data-needs="local"`)}
<p class="fl-help lw-group-help" data-needs="local">Models are never downloaded automatically: press Download, or put them in <code id="models_dir" class="lw-mono"></code>.</p>

${SERVER_MODELS_GROUP}

<section class="lw-section" id="or_stt_group" data-needs="cloud">
  <h3 class="fl-group-title lw-title-badge">Cloud · OpenRouter <span class="fl-badge fl-badge-light" id="or_stt_used_badge">Used only when on above</span> <button type="button" class="lw-link lw-link-sm" data-go="settings-openrouter">Change provider →</button></h3>
  <div class="fl-group" id="or_stt_box">
    ${row(`<span id="or_stt_model_name"></span> <span id="or_stt_model_badges" class="pills"></span>`, `<span id="or_stt_model_desc"></span> <code id="or_stt_model_id" class="lw-mono"></code>`, `<button type="button" class="fl-btn fl-btn-secondary lw-btn-160" id="or_stt_change">Change</button>`)}
    ${row("Language", "", `<input id="or_stt_language" class="fl-input lw-w160" spellcheck="false" placeholder="Auto-detect" aria-label="Language (empty = auto-detect)" />`)}
    ${row("Speech path", "A chat model with audio input may paraphrase instead of writing down what you said.", `<select id="or_stt_path" class="fl-select is-wide">
        <option value="auto">Auto (from the catalogue)</option>
        <option value="transcriptions">Transcription endpoint</option>
        <option value="chat_audio">Chat model with audio input</option>
      </select>`)}
    ${row("Send dictionary words", "Helps it spell names the way you do.", toggle("or_stt_send_dictionary", "Send dictionary words"))}
    ${row("Try it", `<span id="or_stt_consent"></span> <span id="or_stt_test_out"></span> <span id="or_stt_status"></span>`, `<button type="button" class="fl-btn fl-btn-secondary" id="or_stt_test">Test</button>`)}
  </div>
</section>

${group("", row("Transcribe while I speak", "Each phrase is turned into text as you pause, so long dictations finish about a second after you let go. Off = the whole recording is transcribed after you stop. The cloud always gets the whole recording.", toggle("dev_decode_while_speaking", "Transcribe while I speak", `data-save="dev"`)), `data-needs="dev"`)}`,
};
