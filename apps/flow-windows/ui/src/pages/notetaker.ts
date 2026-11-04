// Notetaker preview page (PageNotetaker.dc.html): the static "coming
// later" page shown while the Notetaker is hidden (D51). The working
// Meetings page (pages/meetings.ts) and the shell's recorder are kept;
// NOTETAKER_ENABLED picks which one the app window shows.

import { ICON, iconSvg } from "../lib/icons.js";
import type { Page } from "../lib/router.js";

/** The Notetaker switch, UI half. Must equal `meetings::NOTETAKER_ENABLED`
 *  in src/meetings/mod.rs (a Rust test checks this line). `false` = the
 *  preview page below, no Meetings page, no meeting shortcut row, no
 *  Meetings group under Speech to text. */
export const NOTETAKER_ENABLED = false;

const TEMPLATE = `
<div class="lw-page lw-nt">
  <div class="lw-page-head">
    <h1 class="fl-page-title">Notetaker</h1>
    <span class="fl-badge fl-badge-light">Coming later</span>
  </div>
  <div class="lw-nt-hero">
    ${iconSvg(ICON.notetaker, 24)}
    <h2 class="fl-serif lw-nt-title">Meeting notes, <em>without the bot</em></h2>
    <p class="lw-nt-body">Notetaker will record your mic and computer audio, then transcribe and summarise the meeting with the same engines you use for dictation. No bot joins the call, and nothing leaves this PC unless you've turned on OpenRouter.</p>
    <ul class="lw-nt-points">
      <li>Works with any meeting app</li>
      <li>Summary and action items from AI cleanup</li>
      <li>Notes saved as files you own</li>
    </ul>
  </div>
  <p class="fl-help">Not available yet: this page shows what is planned.</p>
</div>`;

/** A static preview of the hidden Notetaker; it calls no command. */
export const notetakerPage: Page = {
  id: "notetaker",
  area: "main",
  template: TEMPLATE,
};
