// A feature page that is locked while the feature is opted out: its
// controls sit in a `<fieldset>` that is disabled (greyed out, still
// shown, nothing saved or deleted), and a banner at the top says the
// feature is off, with a button that turns it on. Turning it on (from
// the banner or the page's own switch) unlocks the page at once: the
// fieldset only overrides the controls' own disabled states, so they
// come back as the page left them. First used by Transforms.
// No DOM access at import, so tests/opt-out-lock.test.mjs runs it in node.

/**
 * The banner and the locked region's markup; `id` prefixes their ids.
 * Every argument goes into the HTML as is: pass the page's own constant
 * text only, never user data.
 */
export function optOutMarkup(id: string, feature: string, body: string): { banner: string; open: string; close: string } {
  return {
    banner:
      `<div class="lw-optout" id="${id}_off" hidden>` +
      `<p class="lw-optout-text" role="status"><strong>${feature} is off.</strong> ${body}</p>` +
      `<button type="button" class="fl-btn fl-btn-primary fl-btn-sm" id="${id}_turn_on">Turn on ${feature}</button>` +
      `</div>`,
    open: `<fieldset class="lw-optout-region" id="${id}_region" aria-label="${feature} settings">`,
    close: `</fieldset>`,
  };
}

/** What the lock touches: the banner and the fieldset around the controls. */
export interface OptOutView {
  banner: { hidden: boolean };
  region: { disabled: boolean };
}

/** Show the page for the feature being on (unlocked) or off (locked). */
export function applyOptOut(view: OptOutView, on: boolean): void {
  view.banner.hidden = on;
  view.region.disabled = !on;
}

interface Clickable {
  addEventListener(type: "click", fn: () => void): void;
}

export interface OptOutWiring {
  /** The feature's usual way of turning on (its consent flow). */
  turnOn: () => void;
  /** The feature's own on / off switch: focus moves there after "Turn on"
   *  (the banner and its button go away). */
  switchId: string;
}

/** Find the pieces `optOutMarkup(id, …)` made and wire the banner's button.
 *  Returns `apply`, for the page's render to call with the feature's
 *  current state. */
export function wireOptOut(id: string, find: (id: string) => unknown, w: OptOutWiring): (on: boolean) => void {
  const view: OptOutView = {
    banner: find(`${id}_off`) as OptOutView["banner"],
    region: find(`${id}_region`) as OptOutView["region"],
  };
  (find(`${id}_turn_on`) as Clickable).addEventListener("click", () => {
    w.turnOn();
    (find(w.switchId) as { focus(): void }).focus();
  });
  return (on) => applyOptOut(view, on);
}
