// Client of ps/agent.ps1 (through the SSH tunnel): one JSON line per request.
import { connect } from "node:net";
import { AGENT_PORT } from "./ports.mjs";

export class Agent {
  constructor(port = AGENT_PORT) {
    this.port = port;
    this.stopped = false;
  }

  /** After a signal: every later call fails at once (except `force` ones, e.g. quit). */
  stop() {
    this.stopped = true;
  }

  /** Send `{cmd, ...args}`; resolves with the reply, throws on {ok:false}. */
  call(cmd, args = {}, timeoutMs = 60_000, { force = false } = {}) {
    if (this.stopped && !force) return Promise.reject(new Error(`agent ${cmd}: run interrupted`));
    return new Promise((resolve, reject) => {
      const s = connect(this.port, "127.0.0.1");
      let buf = "";
      const t = setTimeout(() => {
        s.destroy();
        reject(new Error(`agent ${cmd}: timeout`));
      }, timeoutMs);
      s.setEncoding("utf8");
      s.on("connect", () => s.write(JSON.stringify({ cmd, ...args }) + "\n"));
      s.on("data", (d) => (buf += d));
      s.on("error", (e) => {
        clearTimeout(t);
        reject(new Error(`agent ${cmd}: ${e.message}`));
      });
      s.on("close", () => {
        clearTimeout(t);
        let r;
        try {
          r = JSON.parse(buf.replace(/^﻿/, ""));
        } catch {
          return reject(new Error(`agent ${cmd}: bad reply ${JSON.stringify(buf.slice(0, 200))}`));
        }
        if (!r.ok) return reject(new Error(`agent ${cmd}: ${r.error}`));
        resolve(r);
      });
    });
  }

  /** Wait until the agent answers (it takes a few seconds to compile its C#). */
  async waitUp(timeoutMs = 45_000) {
    const end = Date.now() + timeoutMs;
    let last;
    while (Date.now() < end) {
      try {
        return await this.call("ping", {}, 5000);
      } catch (e) {
        last = e;
        await new Promise((r) => setTimeout(r, 700));
      }
    }
    throw new Error(`agent did not come up: ${last?.message}`);
  }
}
