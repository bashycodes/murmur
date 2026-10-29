// Developer: hybrid local/server routing (docs/hybrid-architecture.md §5),
// speech decoding, the opt-in debug log and engine status. The backend
// owns validation (`dev_validate`), so the inline errors and Save use the
// same rules; the token is shown here but stored DPAPI-encrypted.

import { $, button, el, html, setStatus } from "../lib/dom.js";
import { cloudHint } from "../lib/format.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { router } from "../lib/router.js";
import type { DevCheck, DevForm, DevView, FieldError, Health, LastDictation, Route, RouteView } from "../lib/types.js";
import { views } from "../lib/views.js";

interface SessionTest {
  status: string;
  empty_reason: string | null;
  error: string | null;
  engine: string | null;
  fallback: string | null;
  total_ms: number;
}

interface DebugLogView {
  enabled: boolean;
  count: number | null;
  path: string;
  stats: { written: number; dropped: number; write_errors: number };
}

const TEMPLATE = `
<section class="panel" data-panel="developer">
  <fieldset>
    <legend>Engines</legend>
    <p class="hint">Where speech is turned into text. <strong>Client</strong> = this PC. <strong>Server</strong> = your own flow-server (your LAN or a VPN); audio goes only there, never to a third party.<span id="dev_or_hint" hidden> <strong>OpenRouter</strong> = cloud speech-to-text, turned on and set up in the OpenRouter tab; only its place in the order is set here.</span></p>
    <label class="row"><input type="checkbox" id="dev_allow_server" /> Allow server <small id="dev_server_na" class="hint" hidden>(not in this build)</small></label>
    <label class="row"><input type="checkbox" id="dev_allow_client" /> Allow client (this PC) <small id="dev_client_na" class="hint" hidden>(not in this build)</small></label>
    <div id="dev_order_box">
      <p class="hint">Try in this order (the next one is the fallback):</p>
      <ol id="dev_routes" class="routes"></ol>
    </div>
    <label class="row"><input type="checkbox" id="dev_keep_local_warm" /> Keep the local model loaded while the server is preferred (instant fallback)</label>
    <label class="row"><input type="checkbox" id="dev_show_engine_badge" /> Show S / L on the Flow bar after each dictation (red dot = fell back)</label>
    <ul id="dev_warnings" class="warnings"></ul>
  </fieldset>
  <fieldset>
    <legend>Speech decoding</legend>
    <label class="row"><input type="checkbox" id="dev_decode_while_speaking" /> Decode while speaking</label>
    <p class="hint">Transcribes each finished phrase while you keep talking, so a long dictation is ready about as fast as a short one (cuts go in pauses of 0.8 s or more). Off = the whole recording is transcribed after you stop. OpenRouter always gets the whole recording at the end.</p>
    <label>Parakeet decoding
      <select id="dev_parakeet_decoding">
        <option value="auto">Auto (default): beam search with dictionary words, greedy otherwise</option>
        <option value="greedy">Greedy: one recognizer, ~0.6 GB less memory, no dictionary biasing</option>
      </select>
    </label>
    <p class="hint">Parakeet only (whisper ignores it). Changing it reloads the model.</p>
  </fieldset>

  <fieldset id="dev_server_box">
    <legend>Server</legend>
    <div class="grid2">
      <label>Host or IP <input id="dev_host" spellcheck="false" placeholder="100.64.0.10 or mybox.local" /><small class="field-err" data-err="host"></small></label>
      <label>Port <input id="dev_port" type="number" min="1" max="65535" /><small class="field-err" data-err="port"></small></label>
    </div>
    <label class="row"><input type="checkbox" id="dev_tls" /> Use TLS (wss) <small id="dev_tls_forced" class="hint" hidden>— required for a public address</small></label>
    <label id="dev_pin_row">Certificate pin (SHA-256, optional) <input id="dev_tls_pin_sha256" spellcheck="false" placeholder="64 hex characters" /><small class="field-err" data-err="tls_pin_sha256"></small></label>
    <label>Token
      <span class="token-row">
        <input id="dev_token" type="password" autocomplete="off" spellcheck="false" placeholder="from flow-server gen-token" />
        <button type="button" class="small secondary" id="dev_token_show">Show</button>
        <button type="button" class="small secondary" id="dev_token_paste">Paste</button>
        <button type="button" class="small secondary" id="dev_token_clear">Clear</button>
      </span>
      <small class="field-err" data-err="token"></small>
    </label>
    <p class="hint">Stored encrypted for your Windows account (DPAPI); never logged.</p>
    <div class="grid3">
      <label>Connect timeout (ms) <input id="dev_connect_timeout_ms" type="number" min="100" max="10000" step="100" /><small class="field-err" data-err="connect_timeout_ms"></small></label>
      <label>Commit timeout base (ms) <input id="dev_commit_timeout_base_ms" type="number" min="500" max="60000" step="100" /><small class="field-err" data-err="commit_timeout_base_ms"></small></label>
      <label>Health cache (s) <input id="dev_health_ttl_s" type="number" min="5" max="600" /><small class="field-err" data-err="health_ttl_s"></small></label>
    </div>
  </fieldset>

  <div class="toolbar">
    <button type="button" id="dev_save">Save</button>
    <button type="button" id="dev_test_conn" class="secondary">Test connection</button>
    <button type="button" id="dev_test_session" class="secondary">Test session</button>
    <button type="button" id="dev_reset_circuit" class="secondary">Reset circuit</button>
    <button type="button" id="dev_reset" class="secondary danger">Reset to defaults</button>
    <button type="button" id="dev_lock" class="secondary" hidden>Lock</button>
  </div>
  <p id="dev_status" class="hint"></p>
  <p id="dev_test" class="hint"></p>

  <!-- Opt-in debug log (README "Debug log"): off by default, saved at once. -->
  <fieldset id="dev_debug_box">
    <legend>Debug log</legend>
    <label class="row"><input type="checkbox" id="dev_debug_log" /> Debug log (includes transcripts)</label>
    <p class="hint warn">When on, each dictation's raw and final text, timings, engine and target app are written to <code id="dev_debug_path"></code> (last 500) and printed to the console. It stays on this PC and never contains your server token, API keys or clipboard. Turn it off when you are done.</p>
    <p class="hint" id="dev_debug_count">No entries</p>
    <div class="toolbar">
      <button type="button" class="small secondary" id="dev_debug_open">Open log folder</button>
      <button type="button" class="small secondary danger" id="dev_debug_clear">Clear</button>
    </div>
  </fieldset>

  <dialog id="dev_debug_off">
    <h3>Debug log turned off</h3>
    <p id="dev_debug_off_body">The log still holds transcripts of earlier dictations. Delete it now?</p>
    <div class="toolbar">
      <button type="button" class="secondary" id="dev_debug_keep">Keep the file</button>
      <button type="button" class="danger" id="dev_debug_delete">Delete the log</button>
    </div>
  </dialog>

  <fieldset>
    <legend>Status</legend>
    <dl class="kv">
      <dt>Build</dt><dd id="dev_build">…</dd>
      <dt>Active engine (last dictation)</dt><dd id="dev_last">No dictation yet</dd>
      <dt>Server info</dt><dd id="dev_server_info">Not probed yet</dd>
      <dt>Circuit</dt><dd id="dev_circuit">closed</dd>
      <dt id="dev_cloud_circuit_dt" hidden>Cloud circuit</dt><dd id="dev_cloud_circuit" hidden>closed</dd>
    </dl>
  </fieldset>
</section>`;

const ROUTE_LABEL: Record<Route, string> = {
  server: "Server",
  client: "This PC (client)",
  cloud: "OpenRouter speech-to-text (cloud)",
};
const DEV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"] as const;
const DEV_TEXT = ["host", "tls_pin_sha256", "token"] as const;
const DEV_BOOL = [
  "allow_server",
  "allow_client",
  "tls",
  "keep_local_warm",
  "show_engine_badge",
  "decode_while_speaking",
] as const;

let devOrder: Route[] = ["server", "client", "cloud"];
let devValid = true;

const HINTS: Record<string, string> = {
  auth_failed: "token rejected — copy it again from the server's token file",
  connect_failed: "is flow-server running / LAN or VPN up?",
  proto_mismatch: "client and server protocol versions differ — update one of them",
  server_not_configured: "enter the server host first",
  timeout: "the server is too slow or stuck",
  busy: "the server is busy with another dictation",
};

function hint(code: string | null): string {
  return code ? (HINTS[code] ?? "") : "";
}

function readDev(): DevForm {
  const base = views.dev?.form;
  const f = { ...(base as DevForm) };
  for (const k of DEV_BOOL) f[k] = $<HTMLInputElement>(`dev_${k}`).checked;
  for (const k of DEV_TEXT) f[k] = $<HTMLInputElement>(`dev_${k}`).value;
  for (const k of DEV_NUM) {
    const v = $<HTMLInputElement>(`dev_${k}`).value.trim();
    f[k] = v === "" ? -1 : Number(v);
  }
  f.route_order = [...devOrder];
  f.parakeet_decoding = $<HTMLSelectElement>("dev_parakeet_decoding").value || f.parakeet_decoding || "auto";
  // D28: the cloud settings live on the OpenRouter tab; never sent from here.
  delete (f as Partial<Record<string, unknown>>).cloud;
  delete (f as Partial<Record<string, unknown>>).allow_cloud;
  return f;
}

function fillDev(f: DevForm): void {
  for (const k of DEV_BOOL) $<HTMLInputElement>(`dev_${k}`).checked = f[k];
  for (const k of DEV_TEXT) $<HTMLInputElement>(`dev_${k}`).value = f[k];
  for (const k of DEV_NUM) $<HTMLInputElement>(`dev_${k}`).value = String(f[k]);
  showTokenSaved(!!f.token_saved);
  devOrder = [...f.route_order];
  $<HTMLSelectElement>("dev_parakeet_decoding").value = f.parakeet_decoding ?? "auto";
  renderRoutes();
}

// The saved token never comes back to the page (S18): an empty field
// keeps it; typing replaces it; Clear removes it on Save.
function showTokenSaved(saved: boolean): void {
  $<HTMLInputElement>("dev_token").placeholder = saved
    ? "saved (hidden) — leave empty to keep it"
    : "from flow-server gen-token";
}

function renderRoutes(): void {
  const srv = $<HTMLInputElement>("dev_allow_server").checked;
  const cli = $<HTMLInputElement>("dev_allow_client").checked;
  // Cloud STT is switched on in the OpenRouter tab; only its place in the order is set here.
  const cloud = !!views.or?.stt.allow && !!views.dev?.caps.cloud;
  const on = (r: Route): boolean => (r === "server" ? srv : r === "client" ? cli : cloud);
  // Cloud is listed only in builds that have it.
  const shown = devOrder.filter((r) => r !== "cloud" || !!views.dev?.caps.cloud);
  $<HTMLElement>("dev_order_box").hidden = shown.filter(on).length < 2;
  $<HTMLOListElement>("dev_routes").replaceChildren(
    ...shown.map((r, i) => {
      const li = el("li", on(r) ? "" : "off", ROUTE_LABEL[r] + " ");
      if (i > 0) {
        li.append(
          button("Move up", () => {
            const a = devOrder.indexOf(shown[i - 1]);
            const b = devOrder.indexOf(r);
            [devOrder[a], devOrder[b]] = [devOrder[b], devOrder[a]];
            renderRoutes();
            scheduleCheck();
          }, "small secondary"),
        );
      }
      return li;
    }),
  );
  $<HTMLElement>("dev_server_box").hidden = !srv;
  $<HTMLElement>("dev_pin_row").hidden = !$<HTMLInputElement>("dev_tls").checked;
}

function showCheck(c: DevCheck): void {
  for (const s of document.querySelectorAll<HTMLElement>("[data-err]")) s.textContent = "";
  for (const i of document.querySelectorAll<HTMLInputElement>("[data-panel=developer] input")) i.classList.remove("invalid");
  for (const e of c.errors) {
    const slot = document.querySelector<HTMLElement>(`[data-err="${e.field}"]`);
    if (slot) slot.textContent = e.message;
    $<HTMLInputElement>(`dev_${e.field}`)?.classList.add("invalid");
  }
  devValid = c.errors.length === 0;
  $<HTMLButtonElement>("dev_save").disabled = !devValid;
  $<HTMLButtonElement>("dev_test_conn").disabled = !devValid;
  $<HTMLElement>("dev_tls_forced").hidden = !c.tls_forced;
  const tls = $<HTMLInputElement>("dev_tls");
  if (c.tls_forced) tls.checked = true;
  tls.disabled = c.tls_forced;
  $<HTMLUListElement>("dev_warnings").replaceChildren(
    ...c.warnings.map((w) => el("li", w.startsWith("Dictation disabled") ? "err" : "", w)),
  );
}

let checkTimer = 0;
function scheduleCheck(): void {
  renderRoutes();
  clearTimeout(checkTimer);
  checkTimer = setTimeout(() => {
    void invoke<DevCheck>("dev_validate", { form: readDev() }).then(showCheck);
  }, 150);
}

function healthText(h: Health): string {
  if (!h.ok) {
    const code = h.error ?? "error";
    // flow-remote refuses ws:// to an address that is not private/VPN (S6).
    if (/plaintext (refused|to public address)/.test(h.message ?? "")) {
      return "Server needs TLS — its address is not private (or resolves to a public one); turn on TLS and set a certificate pin";
    }
    const why = hint(code);
    return `${code}${h.message ? `: ${h.message}` : ""}${why ? ` — ${why}` : ""}`;
  }
  const r = h.rtt_ms;
  const parts = [
    r ? `OK · median ${r.median} ms (${r.min}-${r.max})` : "OK",
    h.server_version ?? "",
    h.asr_backend ?? "",
  ].filter(Boolean);
  return parts.join(" · ");
}

function showServerInfo(h: Health | null): void {
  const e = $<HTMLElement>("dev_server_info");
  if (!h) {
    e.textContent = "Not probed yet";
    e.className = "";
    return;
  }
  if (!h.ok) {
    e.textContent = healthText(h);
    e.className = "err";
    return;
  }
  const pp = h.postprocess_match === false ? " · ⚠ server pipeline ≠ client pipeline; results may differ" : " · pipeline ✓";
  e.textContent =
    `${h.server ?? ""} · ${h.server_version ?? "?"} (core ${h.server_core ?? "?"}, proto ${h.proto ?? "?"})` +
    ` · ASR ${h.asr_backend ?? "?"} ${h.asr_model ?? ""} · LLM ${h.llm ? "on" : "off"}` +
    ` · workers ${h.workers ?? "?"} (busy ${h.busy ?? "?"})${pp}`;
  e.className = h.postprocess_match === false ? "err" : "ok";
}

/** Shared with Android (D22): `total {client_ms ?: total_ms} ms · asr {asr_ms} (+{pre_asr_ms} early) · wait {wait_ms} · {segments} seg`. */
function timingLine(l: LastDictation): string {
  const parts = [`total ${l.client_ms ?? l.total_ms} ms`];
  parts.push(l.pre_asr_ms > 0 ? `asr ${l.asr_ms} (+${l.pre_asr_ms} early)` : `asr ${l.asr_ms}`);
  if (l.wait_ms > 0) parts.push(`wait ${l.wait_ms}`);
  if (l.segments > 0) parts.push(`${l.segments} seg`);
  if (l.client_ms != null && l.client_ms !== l.total_ms) parts.push(`engine ${l.total_ms}`);
  return parts.join(" · ");
}

function showRoute(r: RouteView): void {
  const last = r.last;
  const e = $<HTMLElement>("dev_last");
  if (!last) {
    e.textContent = "No dictation yet";
  } else {
    const engine =
      last.engine === "remote" ? "server" : last.engine === "local" ? "client" : (last.engine ?? "none");
    const bits = [
      last.row ?? engine,
      last.notice ?? "",
      last.fallback ? `fallback: ${last.fallback}` : "",
      last.server ? `server ${last.server}` : "",
      last.rtt_ms != null ? `rtt ${last.rtt_ms} ms` : "",
      last.queue_ms != null ? `queue ${last.queue_ms} ms` : "",
      timingLine(last),
      last.status,
      last.empty_reason ?? "",
      last.llm_row ?? last.llm ?? "",
      new Date(last.at_ms).toLocaleTimeString(),
    ].filter(Boolean);
    e.textContent = bits.join(" · ");
  }
  const c = r.circuit;
  $<HTMLElement>("dev_circuit").textContent = !c.open
    ? "closed"
    : c.until_config_change
      ? `open until the server settings change (${c.reason ?? "?"})`
      : `open until ${new Date(Date.now() + (c.retry_in_ms ?? 0)).toLocaleTimeString()} (${c.reason ?? "?"})`;
  showServerInfo(r.health);
  const cc = r.cloud_circuit;
  $<HTMLElement>("dev_cloud_circuit_dt").hidden = !cc;
  $<HTMLElement>("dev_cloud_circuit").hidden = !cc;
  if (cc) {
    $<HTMLElement>("dev_cloud_circuit").textContent = !cc.open
      ? "closed"
      : cc.until_config_change
        ? `open until the key, model or privacy settings change (${cloudHint(cc.reason)})`
        : `open until ${new Date(Date.now() + (cc.retry_in_ms ?? 0)).toLocaleTimeString()} (${cloudHint(cc.reason)})`;
  }
}

function showDev(v: DevView): void {
  views.dev = v;
  fillDev(v.form);
  $<HTMLInputElement>("dev_allow_server").disabled = !v.caps.remote;
  $<HTMLInputElement>("dev_allow_client").disabled = !v.caps.local;
  $<HTMLElement>("dev_server_na").hidden = v.caps.remote;
  $<HTMLElement>("dev_client_na").hidden = v.caps.local;
  $<HTMLElement>("dev_lock").hidden = v.caps.remote;
  $<HTMLElement>("dev_or_hint").hidden = !v.caps.cloud;
  const c = v.capabilities;
  $<HTMLElement>("dev_build").textContent =
    `Murmur ${v.full} · ` +
    `${[c.local ? "client" : "", c.remote ? "server" : ""].filter(Boolean).join(" + ") || "no engine"}` +
    ` · ASR ${c.asr_engines.join(", ") || "none"} · LLM ${c.llm ? "yes" : "no"} · proto ${c.proto} · core ${c.core_version}`;
  showCheck(v.check);
  showRoute(v.route);
  views.setDev(v);
}

async function loadDev(): Promise<void> {
  try {
    showDev(await invoke<DevView>("dev_get"));
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
}

async function devCall<T>(cmd: string, args: Record<string, unknown>, okText: string): Promise<T | null> {
  try {
    const r = await invoke<T>(cmd, args);
    setStatus("dev_status", okText, true);
    return r;
  } catch (e) {
    const errs = e as FieldError[] | string;
    if (Array.isArray(errs)) {
      showCheck({ errors: errs, warnings: [], tls_forced: false });
      setStatus("dev_status", "Fix the highlighted fields", false);
    } else {
      setStatus("dev_status", String(e), false);
    }
    return null;
  }
}

async function devSave(): Promise<void> {
  if (!devValid) return;
  const v = await devCall<DevView>("dev_set", { form: readDev() }, "Saved — applies to the next dictation");
  if (v) showDev(v);
}

async function devTestConnection(): Promise<void> {
  if (!devValid) return;
  const out = $<HTMLElement>("dev_test");
  out.textContent = "Testing…";
  out.className = "hint";
  try {
    const h = await invoke<Health>("dev_test_connection", { form: readDev() });
    out.textContent = `Test connection: ${healthText(h)}`;
    out.className = h.ok ? "hint ok" : "hint err";
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
}

async function devTestSession(): Promise<void> {
  const out = $<HTMLElement>("dev_test");
  out.textContent = "Running a 1 s silent session through the saved settings…";
  out.className = "hint";
  try {
    const r = await invoke<SessionTest>("dev_test_session");
    const engine = r.engine === "remote" ? "server" : r.engine === "local" ? "client" : "none";
    const ok = r.status === "EMPTY" && r.empty_reason === "no_speech";
    out.textContent =
      `Test session: ${r.status}${r.empty_reason ? ` (${r.empty_reason})` : ""} via ${engine}` +
      `${r.fallback ? ` (fallback: ${r.fallback} — ${hint(r.fallback)})` : ""} in ${r.total_ms} ms` +
      `${r.error ? ` — ${r.error}` : ""}`;
    out.className = ok ? "hint ok" : "hint err";
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
  void refreshRoute();
}

async function refreshRoute(): Promise<void> {
  try {
    showRoute(await invoke<RouteView>("dev_route_status"));
  } catch {
    /* tab not loaded yet */
  }
}

let versionTaps = 0;
let versionTimer = 0;
/** Seven taps on the version line unlock this tab. */
export function versionTapped(): void {
  const dv = views.dev;
  if (!dv || dv.visible) return;
  versionTaps++;
  clearTimeout(versionTimer);
  versionTimer = setTimeout(() => (versionTaps = 0), 3000);
  if (versionTaps >= 7) {
    versionTaps = 0;
    void devCall<DevView>("dev_unlock", { unlocked: true }, "Developer settings unlocked").then((v) => {
      if (v) {
        showDev(v);
        router.go("developer");
      }
    });
  }
}

// ------------------------------------------------------------- debug log

function showDebugLog(v: DebugLogView): void {
  $<HTMLInputElement>("dev_debug_log").checked = v.enabled;
  $<HTMLElement>("dev_debug_path").textContent = v.path;
  const n = v.count;
  let line = n === null ? "Log file not readable" : n === 0 ? "No entries" : n === 1 ? "1 entry" : `${n} entries`;
  if (v.stats.write_errors > 0) line += ` · ${v.stats.write_errors} write errors (ignored)`;
  if (v.stats.dropped > 0) line += ` · ${v.stats.dropped} dropped`;
  $<HTMLElement>("dev_debug_count").textContent = line;
}

async function loadDebugLog(): Promise<void> {
  try {
    showDebugLog(await invoke<DebugLogView>("debug_log_status"));
  } catch (e) {
    setStatus("dev_status", String(e), false);
  }
}

/** Turning the log off offers to delete what it already holds. */
function askClearDebugLog(count: number): Promise<boolean> {
  const d = $<HTMLDialogElement>("dev_debug_off");
  $<HTMLElement>("dev_debug_off_body").textContent =
    `The log still holds ${count === 1 ? "1 entry" : `${count} entries`} with transcripts of earlier dictations. Delete it now?`;
  return new Promise((resolve) => {
    const done = (ok: boolean): void => {
      $<HTMLButtonElement>("dev_debug_delete").onclick = null;
      $<HTMLButtonElement>("dev_debug_keep").onclick = null;
      d.close();
      resolve(ok);
    };
    $<HTMLButtonElement>("dev_debug_delete").onclick = () => done(true);
    $<HTMLButtonElement>("dev_debug_keep").onclick = () => done(false);
    d.oncancel = () => done(false);
    d.showModal();
  });
}

async function debugLogToggled(): Promise<void> {
  const on = $<HTMLInputElement>("dev_debug_log").checked;
  const v = await devCall<DebugLogView>(
    "debug_log_set",
    { enabled: on },
    on ? "Debug log on: transcripts are being written" : "Debug log off",
  );
  if (!v) {
    $<HTMLInputElement>("dev_debug_log").checked = !on;
    return;
  }
  showDebugLog(v);
  if (!on && (v.count ?? 0) > 0 && (await askClearDebugLog(v.count ?? 0))) {
    const c = await devCall<DebugLogView>("debug_log_clear", {}, "Debug log off and deleted");
    if (c) showDebugLog(c);
  }
}

export const developerPage = {
  id: "developer",
  mount(root: HTMLElement): void {
    root.append(html(TEMPLATE));
    for (const i of document.querySelectorAll<HTMLInputElement>("[data-panel=developer] input")) {
      i.addEventListener("input", scheduleCheck);
      i.addEventListener("change", scheduleCheck);
    }
    $<HTMLButtonElement>("dev_save").addEventListener("click", () => void devSave());
    $<HTMLInputElement>("dev_debug_log").addEventListener("change", () => void debugLogToggled());
    $<HTMLButtonElement>("dev_debug_open").addEventListener("click", () =>
      void devCall<null>("debug_log_open_folder", {}, "Opened the log folder"),
    );
    $<HTMLButtonElement>("dev_debug_clear").addEventListener("click", () =>
      void devCall<DebugLogView>("debug_log_clear", {}, "Debug log deleted").then((v) => v && showDebugLog(v)),
    );
    $<HTMLButtonElement>("dev_test_conn").addEventListener("click", () => void devTestConnection());
    $<HTMLButtonElement>("dev_test_session").addEventListener("click", () => void devTestSession());
    $<HTMLButtonElement>("dev_reset_circuit").addEventListener("click", () =>
      void devCall<RouteView>("dev_reset_circuit", {}, "Circuit reset").then((r) => r && showRoute(r)),
    );
    $<HTMLButtonElement>("dev_reset").addEventListener("click", () =>
      void devCall<DevView>("dev_reset_defaults", {}, "Defaults restored").then((v) => v && showDev(v)),
    );
    $<HTMLButtonElement>("dev_lock").addEventListener("click", () =>
      void devCall<DevView>("dev_unlock", { unlocked: false }, "Developer settings locked").then((v) => {
        if (v) {
          showDev(v);
          router.go("general");
        }
      }),
    );
    $<HTMLButtonElement>("dev_token_show").addEventListener("click", () => {
      const t = $<HTMLInputElement>("dev_token");
      t.type = t.type === "password" ? "text" : "password";
      $<HTMLButtonElement>("dev_token_show").textContent = t.type === "password" ? "Show" : "Hide";
    });
    $<HTMLButtonElement>("dev_token_paste").addEventListener("click", () => {
      void navigator.clipboard
        .readText()
        .then((t) => {
          $<HTMLInputElement>("dev_token").value = t.trim();
          scheduleCheck();
        })
        .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
    });
    $<HTMLButtonElement>("dev_token_clear").addEventListener("click", () => {
      $<HTMLInputElement>("dev_token").value = "";
      if (views.dev) views.dev.form.token_saved = false;
      showTokenSaved(false);
      scheduleCheck();
    });
    // Turning STT on put `cloud` first in the saved route order (backend):
    // show that order here too.
    views.onOr((v, prev) => {
      if (v?.stt.allow && prev && !prev.stt.allow && v.route_order) devOrder = [...v.route_order];
      renderRoutes();
    });
    void onEvent<RouteView>("dev:route", (e) => showRoute(e.payload));
    void onEvent<null>("history:changed", () => {
      if ($<HTMLInputElement>("dev_debug_log").checked) void loadDebugLog();
    });
  },
  async load(): Promise<void> {
    await loadDev();
    await loadDebugLog();
  },
  show(): void {
    void loadDev();
  },
};
