// The Murmur app on the laptop: launch with the CDP port, stop, and reach
// its webviews (settings window, overlay pill, notes window).
import { Page, listPages, waitPage } from "./cdp.mjs";

export { CDP_PORT } from "./ports.mjs";
import { CDP_PORT } from "./ports.mjs";

export class App {
  /** @param {{remote: import("./remote.mjs").Remote, log: (s:string)=>void}} o */
  constructor({ remote, log }) {
    this.remote = remote;
    this.log = log;
    this.exe = null;
    this.pages = new Map();
  }

  /** Start the app in the interactive session with the debug port open. */
  async launch({ debugPort = CDP_PORT } = {}) {
    if (!this.exe) throw new Error("exe unknown (install first)");
    // The path passes through cmd, powershell -File and the task's own
    // command line: no layer of quoting survives all three.
    if (/\s/.test(this.exe)) throw new Error(`exe path with spaces is not supported: ${this.exe}`);
    this.closePages();
    this.remote.ps("itask.ps1", ["-Script", this.remote.winPath("launch-app.ps1"), "-Arguments", `-Exe ${this.exe} -DebugPort ${debugPort}`, "-Wait"]);
    this.launchedAt = Date.now();
    if (debugPort > 0) await waitPage("settings.html", { port: debugPort, timeoutMs: 45_000 });
  }

  /** Kill the app and its webviews. */
  stop() {
    this.closePages();
    this.remote.ssh("taskkill /F /T /IM murmur-windows.exe", { check: false });
  }

  closePages() {
    for (const p of this.pages.values()) p.close();
    this.pages.clear();
  }

  /** A CDP session on the page whose url contains `part` (cached). */
  async page(part, timeoutMs = 20_000) {
    const cached = this.pages.get(part);
    if (cached && cached.ws.readyState === WebSocket.OPEN) return cached;
    const t = await waitPage(part, { port: CDP_PORT, timeoutMs });
    const p = await Page.open(t);
    this.pages.set(part, p);
    return p;
  }

  settings() {
    return this.page("settings.html");
  }

  overlay() {
    return this.page("overlay.html");
  }

  async pageUrls() {
    try {
      return (await listPages(CDP_PORT)).map((p) => p.url);
    } catch {
      return [];
    }
  }

  /** Call a Tauri command from the settings page. */
  async invoke(cmd, args = {}) {
    const s = await this.settings();
    return s.eval(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`);
  }
}

/**
 * Overlay pill recorder: every change of #pill[data-state] is kept with a
 * timestamp in window.__e2e (installed once per page load).
 */
export const OVERLAY_RECORDER = `(() => {
  if (window.__e2e) return true;
  const pill = document.getElementById("pill");
  const e2e = (window.__e2e = { states: [] });
  const note = () => e2e.states.push({ t: Date.now(), s: pill.dataset.state });
  note();
  new MutationObserver(note).observe(pill, { attributes: true, attributeFilter: ["data-state"] });
  return true;
})()`;
