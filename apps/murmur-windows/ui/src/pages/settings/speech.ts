// Settings → Speech to text (design: SettingsSpeech.dc.html): the engine
// order and the meeting order (D47) (features/engines.ts), the model on
// this PC with its status chip (features/general.ts), live transcription,
// cloud speech to text on OpenRouter or (D85) the LiteLLM proxy
// (features/openrouter.ts), and an Advanced fold with the model plumbing
// (Hebrew model, keep loaded, download source, other models, model file).
// The server's model library is on Settings → Your server. Markup only.

import { ICON, iconSvg } from "../../lib/icons.js";
import { advanced, group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";
import { NOTETAKER_ENABLED } from "../notetaker.js";
import { browseButton, DOWNLOAD_ROW, HEBREW_ROW, SOURCE_ROW } from "../../features/models.js";

/** One engine in a ranked list (`<prefix>_row_<key>`, `_desc_`, `_badge_`):
 *  rank, tile, text, status badge, move up/down, switch. The feature module
 *  moves the row, numbers it and fills the text and badge. `keyAttr` names
 *  the data attribute that holds `key` (e.g. `route`). */
export function engineRow(
  prefix: string,
  keyAttr: string,
  key: string,
  icon: string,
  title: string,
  control: string,
  attrs = "",
): string {
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

export const settingsSpeechPage: Page = {
  id: "settings-speech",
  area: "settings",
  template: `
${heading("Speech to text", "Turns your voice into words. If one engine fails, the next takes over.")}
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
    <div class="fl-row lw-foot-row"><div class="fl-row-desc">Meetings are transcribed after they end, by the first engine that answers. <span id="meet_follow"></span> <span id="meet_order_msg" class="hint"></span></div></div>
  </div>
</section>

${group(
  "",
  row(
    `<label for="asr_engine">This PC model</label>`,
    `Parakeet is English only; Whisper speaks many languages.<span class="lw-status-sub"><span id="model_status"></span></span>`,
    `<select id="asr_engine" class="fl-select lw-model-select" data-save="prefs">
      <option value="parakeet">Parakeet · English, fastest</option>
      <option value="whisper">Whisper · many languages</option>
    </select>
    <span class="fl-badge fl-badge-light" id="pc_model_chip" hidden></span>
    <button type="button" class="fl-btn fl-btn-secondary" id="pc_model_dl" hidden>Download…</button>`,
    `id="pc_model_row"`,
  ),
  `data-needs="local"`,
)}

${group(
  "",
  row(
    "Live transcription",
    "Text appears as you pause, so it finishes sooner.",
    toggle("dev_decode_while_speaking", "Live transcription", `data-save="dev"`),
  ),
  `data-needs="dev"`,
)}

<section class="lw-section" id="or_stt_group" data-needs="cloud">
  <h3 class="fl-group-title lw-title-badge">Cloud · <span id="or_stt_provider_name">OpenRouter</span> <span class="fl-badge fl-badge-light" id="or_stt_used_badge">Used only when on above</span> <button type="button" class="lw-link lw-link-sm" data-go="settings-openrouter">Change provider →</button></h3>
  <div class="fl-group" id="or_stt_box">
    ${row(
      `<span id="or_stt_model_name"></span> <span id="or_stt_model_badges" class="pills"></span>`,
      `<span id="or_stt_model_desc"></span> <code id="or_stt_model_id" class="lw-mono"></code>`,
      `<button type="button" class="fl-btn fl-btn-secondary lw-btn-160" id="or_stt_change">Change</button>`,
      `data-stt-provider="openrouter"`,
    )}
    ${row(
      `<code id="cu_stt_model_name" class="lw-mono"></code>`,
      `Speech model on your proxy at <span id="cu_stt_host" class="lw-mono"></span>. Pick another on the Cloud page.`,
      "",
      `data-stt-provider="custom" hidden`,
    )}
    ${row("Language", "", `<input id="or_stt_language" class="fl-input lw-w160" spellcheck="false" placeholder="Auto-detect" aria-label="Language (empty = auto-detect)" />`, `data-stt-provider="openrouter"`)}
    ${row(
      "Speech path",
      "A chat model with audio input may paraphrase.",
      `<select id="or_stt_path" class="fl-select is-wide">
        <option value="auto">Auto (from the catalogue)</option>
        <option value="transcriptions">Transcription endpoint</option>
        <option value="chat_audio">Chat model with audio input</option>
      </select>`,
      `data-stt-provider="openrouter"`,
    )}
    ${row(
      "Send dictionary words",
      "Helps it spell names your way.",
      toggle("or_stt_send_dictionary", "Send dictionary words"),
      `data-stt-provider="openrouter"`,
    )}
    ${row(
      "Try it",
      `<span id="or_stt_consent"></span> <span id="or_stt_test_out"></span> <span id="or_stt_status"></span>`,
      `<button type="button" class="fl-btn fl-btn-secondary" id="or_stt_test">Test</button>`,
    )}
  </div>
</section>

${advanced(
  HEBREW_ROW +
    row(
      "Keep model loaded",
      "Faster fallback, uses more memory.",
      toggle("dev_keep_local_warm", "Keep model loaded", `data-save="dev"`),
      `data-needs="remote"`,
    ) +
    row(
      "Use dictionary words when listening",
      "Better at names; uses about 0.6 GB more memory.",
      toggle("dev_parakeet_decoding", "Use dictionary words when listening"),
      `data-needs="dev" id="dict_words_row"`,
    ) +
    SOURCE_ROW +
    DOWNLOAD_ROW +
    row(
      "Model file",
      `Empty = the default. Your models folder: <code class="lw-mono" data-models-dir></code>`,
      `<input id="model_path" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" aria-label="Whisper model file" />${browseButton("model_path")}`,
      `data-engine="whisper"`,
    ) +
    row(
      "Model folder",
      `Empty = the default. Your models folder: <code class="lw-mono" data-models-dir></code>`,
      `<input id="parakeet_model_dir" class="fl-input is-wide lw-mono-input" data-save="prefs" spellcheck="false" aria-label="Parakeet model folder" />${browseButton("parakeet_model_dir")}`,
      `data-engine="parakeet"`,
    ),
  `id="stt_advanced" data-needs="local"`,
)}`,
};
