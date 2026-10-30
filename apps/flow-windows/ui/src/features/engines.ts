// Engines: hybrid local/server routing (docs/hybrid-architecture.md §5),
// speech decoding, the server connection and engine status, on the Speech
// to text, System, Your server and Developer settings pages. The backend
// owns validation (`dev_validate`), so the inline errors and Save use the
// same rules; the token is typed here but stored DPAPI-encrypted. Saves go
// through the dev store: switches save at once, the server form on Save.

import { $, el, setStatus } from "../lib/dom.js";
import { devStore } from "../lib/dev-store.js";
import { cloudHint } from "../lib/format.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { router, type Feature } from "../lib/router.js";
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

/** Engine switches that save at once (checkbox id = `dev_<field>`). */
const DEV_SWITCHES = ["allow_server", "allow_client", "keep_local_warm", "show_engine_badge", "decode_while_speaking"] as const;
/** The server form (Your server page), saved with its Save button. */
const SRV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"] as const;
const SRV_TEXT = ["host", "tls_pin_sha256", "token"] as const;

let devOrder: Route[] = ["server", "client", "cloud"];
let devValid = true;
/** The server form has edits that are not saved yet (a switch's save must not overwrite them). */
let serverDirty = false;
/** Clear was pressed: the saved token is removed on Save. */
let tokenCleared = false;

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



/** The Your server page's fields as a form patch. */
function readServer(): Partial<DevForm> {
  const f: Partial<DevForm> = { tls: $<HTMLInputElement>("dev_tls").checked };
  for (const k of SRV_TEXT) f[k] = $<HTMLInputElement>(`dev_${k}`).value;
  for (const k of SRV_NUM) {
    const v = $<HTMLInputElement>(`dev_${k}`).value.trim();
    f[k] = v === "" ? -1 : Number(v);
  }
  if (tokenCleared) f.token_saved = false;
  return f;
}

function fillServer(f: DevForm): void {
  $<HTMLInputElement>("dev_tls").checked = f.tls;
  for (const k of SRV_TEXT) $<HTMLInputElement>(`dev_${k}`).value = f[k];
  for (const k of SRV_NUM) $<HTMLInputElement>(`dev_${k}`).value = String(f[k]);
  tokenCleared = false;
  showTokenSaved(!!f.token_saved);
}

function fillDev(f: DevForm): void {
  for (const k of DEV_SWITCHES) $<HTMLInputElement>(`dev_${k}`).checked = f[k];
  $<HTMLSelectElement>("dev_parakeet_decoding").value = f.parakeet_decoding ?? "auto";
  devOrder = [...f.route_order];
  if (!serverDirty) fillServer(f);
  renderRoutes();
}

// The saved token never comes back to the page (S18): an empty field
// keeps it; typing replaces it; Clear removes it on Save.
function showTokenSaved(saved: boolean): void {
  $<HTMLInputElement>("dev_token").placeholder = saved
    ? "saved (hidden) — leave empty to keep it"
    : "from flow-server gen-token";
}

/** Which engines this build (and the OpenRouter page) offers. */
function available(r: Route): boolean {
  const caps = views.dev?.caps;
  if (r === "server") return !!caps?.remote;
  if (r === "client") return !!caps?.local;
  return !!caps?.cloud && !!views.or?.supported;
}

function engineOn(r: Route): boolean {
  if (r === "server") return $<HTMLInputElement>("dev_allow_server").checked;
  if (r === "client") return $<HTMLInputElement>("dev_allow_client").checked;
  return !!views.or?.stt.allow;
}

function setBadge(r: Route, text: string, kind: "success" | "warning" | "destructive" | "light"): void {
  const b = $<HTMLElement>(`eng_badge_${r}`);
  b.textContent = text;
  b.className = `fl-badge fl-badge-${kind}`;
}

/** One line per engine: what it is and whether it answers. */
function describe(r: Route): void {
  const on = engineOn(r);
  const d = $<HTMLElement>(`eng_desc_${r}`);
  if (r === "server") {
    const f = views.dev?.form;
    const h = views.dev?.route.health;
    const rtt = h?.ok && h.rtt_ms ? ` · ${h.rtt_ms.median} ms` : "";
    d.textContent = f?.host ? `${f.host}${rtt}. Set up under Your server.` : "Not set up yet: add it under Your server.";
    if (!on) setBadge(r, "Off", "light");
    else if (!h) setBadge(r, "On", "light");
    else setBadge(r, h.ok ? "Connected" : "Unreachable", h.ok ? "success" : "destructive");
  } else if (r === "client") {
    const engine = prefsStore.get()?.asr_engine === "parakeet" ? "Parakeet" : "Whisper";
    d.textContent = `${engine}. Private and works offline.`;
    setBadge(r, on ? "On" : "Off", on ? "success" : "light");
  } else {
    const or = views.or;
    d.textContent = on && or ? `${or.stt.model}. Sends your audio to openrouter.ai.` : "Sends your audio to openrouter.ai. Asks first.";
    setBadge(r, on ? "On" : "Off", on ? "success" : "light");
  }
}

function renderRoutes(): void {
  const list = $<HTMLElement>("dev_routes");
  const shown = devOrder.filter(available);
  for (const r of ["server", "client", "cloud"] as Route[]) $<HTMLElement>(`eng_row_${r}`).hidden = !shown.includes(r);
  shown.forEach((r, i) => {
    const row = $<HTMLElement>(`eng_row_${r}`);
    list.append(row); // DOM order = try order
    row.classList.toggle("is-off", !engineOn(r));
    (row.querySelector(".lw-rank") as HTMLElement).textContent = String(i + 1);
    (row.querySelector("[data-move=up]") as HTMLButtonElement).disabled = i === 0;
    (row.querySelector("[data-move=down]") as HTMLButtonElement).disabled = i === shown.length - 1;
    (row.querySelector(".lw-move") as HTMLElement).hidden = shown.length < 2;
    describe(r);
  });
  // Builds with one engine and no server or cloud: nothing to choose.
  $<HTMLElement>("eng_group").hidden = shown.length === 0 || (shown.length === 1 && !views.dev?.visible);
  $<HTMLElement>("dev_pin_row").hidden = !$<HTMLInputElement>("dev_tls").checked;
}

function move(r: Route, dir: -1 | 1): void {
  const shown = devOrder.filter(available);
  const i = shown.indexOf(r);
  const other = shown[i + dir];
  if (!other) return;
  const order = [...devOrder];
  const a = order.indexOf(r);
  const b = order.indexOf(other);
  [order[a], order[b]] = [order[b], order[a]];
  void saveSwitch({ route_order: order });
}

function showCheck(c: DevCheck): void {
  for (const s of document.querySelectorAll<HTMLElement>("[data-err]")) s.textContent = "";
  for (const i of document.querySelectorAll<HTMLInputElement>("[data-dev-field]")) i.classList.remove("invalid");
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
    if (!views.dev) return;
    void invoke<DevCheck>("dev_validate", { form: devStore.form(readServer()) }).then(showCheck);
  }, 150);
}

/** A backend error as text; field errors also go next to their fields. */
function errorText(e: unknown): string {
  const errs = e as FieldError[] | string;
  if (Array.isArray(errs)) {
    showCheck({ errors: errs, warnings: [], tls_forced: false });
    const fields = errs.map((x) => x.message).join("; ");
    return `Your server needs fixing first: ${fields}`;
  }
  return String(e);
}

/** A switch or the order changed: save it alone, over the saved form. */
async function saveSwitch(patch: Partial<DevForm>): Promise<void> {
  try {
    await devStore.save(patch);
    setStatus("status", "Saved — applies to the next dictation", true);
  } catch (e) {
    setStatus("status", errorText(e), false);
    if (views.dev) fillDev(views.dev.form); // back to what is saved
  }
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
  fillDev(v.form);
  $<HTMLElement>("dev_lock").hidden = v.caps.remote;
  const c = v.capabilities;
  $<HTMLElement>("dev_build").textContent =
    `Murmur ${v.full} · ` +
    `${[c.local ? "client" : "", c.remote ? "server" : ""].filter(Boolean).join(" + ") || "no engine"}` +
    ` · ASR ${c.asr_engines.join(", ") || "none"} · LLM ${c.llm ? "yes" : "no"} · proto ${c.proto} · core ${c.core_version}`;
  showCheck(v.check);
  showRoute(v.route);
}

async function loadDev(): Promise<void> {
  try {
    views.setDev(await invoke<DevView>("dev_get"));
  } catch (e) {
    setStatus("status", String(e), false);
  }
}

async function devCall<T>(cmd: string, args: Record<string, unknown>, okText: string, statusId: string): Promise<T | null> {
  try {
    const r = await invoke<T>(cmd, args);
    setStatus(statusId, okText, true);
    return r;
  } catch (e) {
    setStatus(statusId, errorText(e), false);
    return null;
  }
}

async function serverSave(): Promise<void> {
  if (!devValid) return;
  try {
    serverDirty = false;
    await devStore.save(readServer());
    setStatus("dev_status", "Saved — applies to the next dictation", true);
  } catch (e) {
    serverDirty = true;
    setStatus("dev_status", errorText(e), false);
  }
}

async function devTestConnection(): Promise<void> {
  if (!devValid) return;
  const out = $<HTMLElement>("dev_test");
  out.textContent = "Testing…";
  out.className = "hint";
  try {
    const h = await invoke<Health>("dev_test_connection", { form: devStore.form(readServer()) });
    out.textContent = `Test connection: ${healthText(h)}`;
    out.className = h.ok ? "hint ok" : "hint err";
  } catch (e) {
    out.textContent = String(e);
    out.className = "hint err";
  }
}

async function devTestSession(): Promise<void> {
  const out = $<HTMLElement>("dev_diag");
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
    /* not loaded yet */
  }
}

let versionTaps = 0;
let versionTimer = 0;
/** Seven taps on the version line unlock the Developer page. */
export function versionTapped(): void {
  const dv = views.dev;
  if (!dv || dv.visible) return;
  versionTaps++;
  clearTimeout(versionTimer);
  versionTimer = setTimeout(() => (versionTaps = 0), 3000);
  if (versionTaps >= 7) {
    versionTaps = 0;
    void devCall<DevView>("dev_unlock", { unlocked: true }, "Developer settings unlocked", "status").then((v) => {
      if (v) {
        views.setDev(v);
        router.go("settings-developer");
      }
    });
  }
}

export const engines: Feature = {
  init(): void {
    for (const k of DEV_SWITCHES) {
      const box = $<HTMLInputElement>(`dev_${k}`);
      box.addEventListener("change", () => void saveSwitch({ [k]: box.checked }));
    }
    const dec = $<HTMLSelectElement>("dev_parakeet_decoding");
    dec.addEventListener("change", () => void saveSwitch({ parakeet_decoding: dec.value }));
    for (const b of document.querySelectorAll<HTMLButtonElement>("#dev_routes [data-move]")) {
      const r = (b.closest("[data-route]") as HTMLElement).dataset.route as Route;
      b.addEventListener("click", () => move(r, b.dataset.move === "up" ? -1 : 1));
    }
    for (const i of document.querySelectorAll<HTMLInputElement>("[data-dev-field]")) {
      const edited = (): void => {
        serverDirty = true;
        scheduleCheck();
      };
      i.addEventListener("input", edited);
      i.addEventListener("change", edited);
    }
    $<HTMLButtonElement>("dev_save").addEventListener("click", () => void serverSave());
    $<HTMLButtonElement>("dev_test_conn").addEventListener("click", () => void devTestConnection());
    $<HTMLButtonElement>("dev_test_session").addEventListener("click", () => void devTestSession());
    $<HTMLButtonElement>("dev_reset_circuit").addEventListener("click", () =>
      void devCall<RouteView>("dev_reset_circuit", {}, "Circuit reset", "dev_diag_status").then((r) => r && showRoute(r)),
    );
    $<HTMLButtonElement>("dev_reset").addEventListener("click", () =>
      void devCall<DevView>("dev_reset_defaults", {}, "Defaults restored", "dev_diag_status").then((v) => {
        if (!v) return;
        serverDirty = false;
        views.setDev(v);
      }),
    );
    $<HTMLButtonElement>("dev_lock").addEventListener("click", () =>
      void devCall<DevView>("dev_unlock", { unlocked: false }, "Developer settings locked", "status").then((v) => {
        if (v) {
          views.setDev(v);
          router.go("settings-general");
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
          serverDirty = true;
          scheduleCheck();
        })
        .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
    });
    $<HTMLButtonElement>("dev_token_clear").addEventListener("click", () => {
      $<HTMLInputElement>("dev_token").value = "";
      tokenCleared = true;
      serverDirty = true;
      showTokenSaved(false);
      scheduleCheck();
    });
    views.onDev(showDev);
    // Turning STT on put `cloud` first in the saved route order (backend):
    // show that order here too.
    views.onOr((v, prev) => {
      if (v?.stt.allow && prev && !prev.stt.allow && v.route_order) devOrder = [...v.route_order];
      renderRoutes();
    });
    prefsStore.subscribe(() => {
      if (views.dev) renderRoutes();
    });
    void onEvent<RouteView>("dev:route", (e) => {
      showRoute(e.payload);
      if (views.dev) {
        views.dev.route = e.payload;
        renderRoutes();
      }
    });
  },
  load: loadDev,
  shown(page: string, opened: boolean): void {
    if (opened && (page === "settings-speech" || page === "settings-developer")) void refreshRoute();
  },
};
