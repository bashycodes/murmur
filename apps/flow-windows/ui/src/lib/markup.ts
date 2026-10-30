// Template builders for the design's settings layout (fl-group / fl-row).
// Static markup only: every argument is the page's own text or markup,
// never user data.

/** `<div class="fl-row">`: title + optional description on the left, `control` on the right. */
export function row(title: string, desc: string, control: string, attrs = ""): string {
  const d = desc ? `<div class="fl-row-desc">${desc}</div>` : "";
  return `<div class="fl-row"${attrs ? ` ${attrs}` : ""}><div class="fl-row-text"><div class="fl-row-title">${title}</div>${d}</div><div class="fl-row-control">${control}</div></div>`;
}

/** A titled group of rows. */
export function group(title: string, rows: string, attrs = ""): string {
  const t = title ? `<h3 class="fl-group-title">${title}</h3>` : "";
  return `<section class="lw-section"${attrs ? ` ${attrs}` : ""}>${t}<div class="fl-group">${rows}</div></section>`;
}

/** A switch bound to a checkbox `id` (page logic reads `.checked`). */
export function toggle(id: string, label: string, attrs = ""): string {
  return `<input type="checkbox" role="switch" class="fl-toggle" id="${id}" aria-label="${label}"${attrs ? ` ${attrs}` : ""} />`;
}

/** The settings page heading + optional intro. */
export function heading(title: string, intro = ""): string {
  return `<h2 class="fl-modal-title${intro ? " has-intro" : ""}">${title}</h2>${intro ? `<p class="lw-intro">${intro}</p>` : ""}`;
}
