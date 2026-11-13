// Minimal Chrome DevTools Protocol client for the app's WebView2 pages
// (Node's built-in WebSocket; Runtime.evaluate + Page.captureScreenshot).
import { writeFileSync } from "node:fs";
import { CDP_PORT } from "./ports.mjs";

export async function listPages(port = CDP_PORT) {
  const r = await fetch(`http://127.0.0.1:${port}/json/list`);
  return (await r.json()).filter((t) => t.type === "page");
}

/** Wait for a page whose url contains `part`. */
export async function waitPage(part, { port = CDP_PORT, timeoutMs = 30_000 } = {}) {
  const end = Date.now() + timeoutMs;
  let pages = [];
  while (Date.now() < end) {
    try {
      pages = await listPages(port);
      const p = pages.find((x) => x.url.includes(part));
      if (p) return p;
    } catch {
      /* port not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`no page matching ${part} (have: ${pages.map((p) => p.url).join(", ") || "none"})`);
}

export class Page {
  static async open(target) {
    const p = new Page();
    p.target = target;
    p.ws = new WebSocket(target.webSocketDebuggerUrl);
    p.seq = 0;
    p.pending = new Map();
    p.ws.onmessage = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && p.pending.has(m.id)) {
        const { resolve, reject } = p.pending.get(m.id);
        p.pending.delete(m.id);
        if (m.error) reject(new Error(m.error.message));
        else resolve(m.result);
      }
    };
    await new Promise((resolve, reject) => {
      p.ws.onopen = resolve;
      p.ws.onerror = () => reject(new Error(`cannot open ${target.url}`));
    });
    return p;
  }

  send(method, params = {}, timeoutMs = 30_000) {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method}: timeout`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (v) => (clearTimeout(t), resolve(v)),
        reject: (e) => (clearTimeout(t), reject(e)),
      });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  /** Evaluate `expr` (awaits a promise result); returns the JSON value. */
  async eval(expr) {
    const r = await this.send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`eval: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
    return r.result.value;
  }

  async shot(path) {
    const r = await this.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(path, Buffer.from(r.data, "base64"));
    return path;
  }

  /** Poll `expr` until truthy; returns its value, or null after the timeout. */
  async waitFor(expr, timeoutMs = 10_000, everyMs = 200) {
    const end = Date.now() + timeoutMs;
    for (;;) {
      try {
        const v = await this.eval(expr);
        if (v) return v;
      } catch {
        /* page navigating */
      }
      if (Date.now() > end) return null;
      await new Promise((r) => setTimeout(r, everyMs));
    }
  }

  close() {
    try {
      this.ws.close();
    } catch {
      /* already closed */
    }
  }
}
