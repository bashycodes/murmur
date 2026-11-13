// First-run setup (docs/design/app-design/Onboard*.dc.html): a full-window
// overlay over the app shell, shown while prefs.onboarded is false or when
// the user asks for it again (Settings → General). Five steps: welcome,
// where speech runs, the speech model, AI cleanup, try it. Each step saves
// its choice with the existing commands when the user presses Continue
// (dev_set through the dev store, prefs through the prefs store,
// dev_cloud_set_key / dev_or_set_* for OpenRouter); Finish sets
// onboarded = true. Nothing downloads by itself (D9/D31): the model step
// (only when speech runs on this PC, lib/setup-model.ts) offers one
// recommended model and "More languages ▸", each with its state, and
// downloads one after the user pressed Download and confirmed it (3a/3b,
// through features/models.ts); files of the user's own and "Download from"
// (D79) are set in Settings → Speech to text. Step 4 offers "Download from"
// for the cleanup model (picking Your server with none saved shows step 2's
// server fields right there), and "Your server" (D36 server cleanup,
// lib/setup-cleanup.ts) once a server is saved. A server test or an
// OpenRouter key/consent happens only on the user's own click.

import { downloadSource, hasServer, mb, models, pickPath, progressLine, recommendedLlm, setDownloadSource, type CatalogEntry } from "../features/models.js";
import { devStore } from "../lib/dev-store.js";
import { $, el, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { captureChord, stopCapture } from "../lib/keycapture.js";
import { sourceHint, sourceView, type ModelSource, type SourceView } from "../lib/model-source.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { chip, fileState, initialModel, PARAKEET_TITLE, setupSteps, whisperRow, type FileState, type SetupStep, type SpeechRow, type WhisperRow } from "../lib/setup-model.js";
import { changesLine, savedEngine, setupChanges, STEP_OF, type Labels, type SetupChange, type SetupConfig, type SpeechModel } from "../lib/setup-diff.js";
import { pillLine, readinessErrorLine, readinessLine, RECHECK_MS, type PillState, type Readiness, type TryLine, type TryLineSource } from "../lib/setup-try.js";
import type { CleanupEngine, DevForm, DevView, FieldError, Health, KeyState, ModelStatus, OrView, Route, RouteView } from "../lib/types.js";
import { initialEngine, recommendedEngine } from "../lib/setup-engine.js";
import { cloudMissingLine, enginesFor, initialCleanup, recommendedCleanup, serverOption, type ServerFacts, type SetupCleanup } from "../lib/setup-cleanup.js";
import { cloudWhere, providerOf, setupCloudCopy } from "../lib/provider-names.js";
import { views } from "../lib/views.js";
import { engines as cleanupEngines, pollCleanupStatus } from "../features/ai.js";
import { TEMPLATE } from "./templates.js";

type Engine = "pc" | "server" | "cloud";
type Step = SetupStep;

interface LlmStatus {
  state: string;
  path?: string;
  message?: string;
}
interface ModelInfo {
  status: ModelStatus;
  default_path: string;
  default_parakeet_dir: string;
  models_dir: string;
  llm: LlmStatus;
  default_llm_path: string;
}

const ENGINE_ROUTE: Record<Engine, Route> = { pc: "client", server: "server", cloud: "cloud" };
const SAMPLE: Record<SetupCleanup, string> = {
  rules: "So lunch is Thursday.",
  pc: "Lunch is on Thursday.",
  server: "Lunch is on Thursday.",
  cloud: "Lunch is on Thursday.",
};
/** The recommended speech model (model_catalog id). */
const PARAKEET_ID = "parakeet-tdt-0.6b-v2-int8";

const st = {
  step: "welcome" as Step,
  engine: "pc" as Engine,
  cleanup: "rules" as SetupCleanup,
  /** The user picked a cleanup option on this run (else it follows the saved settings and the server). */
  cleanupPicked: false,
  /** Step 2's "I already have a server" is open (the address and token show). */
  serverOpen: false,
  /** Step 2's Recommended card, decided when setup opens (setup-engine.ts). */
  recEngine: null as Engine | null,
  model: null as ModelStatus | null,
  info: null as ModelInfo | null,
  openedAt: 0,
  /** Who set the try step's line (setup-try.ts). */
  line: "none" as TryLineSource,
  /** When a finished dictation's result was last shown (ms). */
  resultAt: 0,
  busy: false,
  /** "Download from" as picked on this run (null = as the backend has it). */
  pick: null as ModelSource | null,
  /** Step 3's picked row (preselected: the model in use). */
  modelPick: "parakeet" as SpeechRow,
  /** Step 3's "More languages" is open. */
  moreOpen: false,
  /** A re-run (setup was finished before): the settings it opened with (null: a first run). */
  rerunSnapshot: null as Before | null,
  /** Server tokens saved on this run (D95 says "Server token replaced"; the token is never read back). */
  tokenSaves: 0,
};

/**
 * The saved settings a re-run opened with (D95): its choices as setup names
 * them, and the values Keep puts back for a choice that changed.
 */
interface Before {
  cfg: SetupConfig;
  dev: Pick<DevForm, "allow_client" | "allow_server" | "route_order">;
  stt: OrView["stt"] | null;
  cleanup: { allow: boolean; consent: boolean; engines: CleanupEngine[] } | null;
  prefs: Partial<Prefs>;
}
let onDone: () => void = () => {};

// ------------------------------------------------------------ capabilities

function has(e: Engine): boolean {
  const caps = views.dev?.caps;
  if (e === "pc") return caps ? caps.local : true;
  if (e === "server") return !!caps?.remote;
  return !!views.or?.supported;
}

/** The server as step 4 sees it: saved in step 2 (`hasServer`) and whether it can clean (D36). */
function serverFacts(): ServerFacts {
  return { build: has("server"), saved: hasServer(), llm: views.or?.cleanup.status.server_llm };
}

function hasCleanup(c: SetupCleanup): boolean {
  if (c === "rules") return true;
  if (c === "pc") return views.dev ? views.dev.caps.local || views.dev.capabilities.llm : true;
  if (c === "server") return !serverOption(serverFacts()).disabled;
  return !!views.or?.supported;
}

/** Step 4's selection from the saved settings (Cloud on, the level, the D36 order). */
function savedCleanup(): SetupCleanup {
  const p = prefsStore.get();
  const c = initialCleanup(
    {
      cloudAllow: !!views.or?.cleanup.allow,
      aiLevel: !!p && p.cleanup_level !== "none",
      llmEnabled: !!p?.llm_enabled,
      engines: cleanupEngines(),
    },
    hasServer(),
  );
  return hasCleanup(c) ? c : "rules";
}

/** Steps shown: the model step only with speech on this PC (and a build that has it). */
function steps(): Step[] {
  return setupSteps(has("pc"), st.engine);
}

// ------------------------------------------------------------ helpers

const panel = (s: Step): HTMLElement => document.querySelector<HTMLElement>(`#onboarding [data-ob-step="${s}"]`) as HTMLElement;

function status(id: string, text: string, ok?: boolean): void {
  const e = $<HTMLElement>(id);
  e.textContent = text;
  e.classList.toggle("ok", ok === true);
  e.classList.toggle("err", ok === false);
}

function nextBtn(s: Step): HTMLButtonElement {
  return panel(s).querySelector<HTMLButtonElement>('[data-ob="next"]') as HTMLButtonElement;
}

function keySet(): boolean {
  return !!views.or?.key.set;
}

/** Fill both "OpenRouter API key" rows: a saved key is not asked for again. */
function showKeyRows(): void {
  for (const r of document.querySelectorAll<HTMLElement>("#onboarding [data-ob-key-row]")) {
    const input = r.querySelector<HTMLInputElement>("[data-ob-key]") as HTMLInputElement;
    const desc = r.querySelector<HTMLElement>("[data-ob-key-desc]") as HTMLElement;
    if (keySet()) {
      input.placeholder = `saved ${views.or?.key.masked ?? ""}`;
      desc.textContent = "A key is saved. Paste a new one only to replace it.";
    }
  }
}

/** Save a typed OpenRouter key; true when a key is (now) saved. */
async function saveKey(inputId: string, statusId: string): Promise<boolean> {
  const typed = $<HTMLInputElement>(inputId).value.trim();
  if (typed) {
    try {
      await invoke<KeyState>("dev_cloud_set_key", { key: typed });
      $<HTMLInputElement>(inputId).value = "";
      views.setOr(await invoke<OrView>("dev_or_get"));
      showKeyRows();
    } catch (e) {
      status(statusId, String(e), false);
      return false;
    }
  }
  if (!keySet()) {
    status(statusId, "Paste your OpenRouter API key first.", false);
    return false;
  }
  return true;
}

async function refreshDev(): Promise<void> {
  views.setDev(await invoke<DevView>("dev_get"));
}

function errText(e: unknown): string {
  if (Array.isArray(e)) return (e as FieldError[]).map((x) => x.message).join("; ");
  return String(e);
}

// ------------------------------------------------------------ step: engine

function pickEngine(e: Engine): void {
  if (!has(e)) return;
  st.engine = e;
  renderEngine();
}

function renderEngine(): void {
  for (const b of document.querySelectorAll<HTMLButtonElement>("#onboarding [data-ob-engine]")) {
    const id = b.dataset.obEngine as Engine;
    const on = id === st.engine;
    b.setAttribute("aria-checked", String(on));
    b.classList.toggle("is-on", on);
    b.disabled = !has(id);
    (b.querySelector("[data-ob-na]") as HTMLElement).hidden = has(id);
    (b.querySelector("[data-ob-rec]") as HTMLElement).hidden = id !== st.recEngine;
  }
  // D96: the cloud card names your own AI server when that is the provider.
  const copy = setupCloudCopy(providerOf(views.or));
  const card = document.querySelector<HTMLElement>('#onboarding [data-ob-engine="cloud"]');
  if (card) {
    (card.querySelector(".lw-ob-card-title") as HTMLElement).textContent = copy.sttTitle;
    (card.querySelector(".lw-ob-card-desc") as HTMLElement).textContent = copy.sttDesc;
    (card.querySelector(".lw-ob-facts") as HTMLElement).replaceChildren(...copy.sttFacts.map((f) => el("span", undefined, f)));
  }
  placeServerForm();
  $<HTMLElement>("ob_cloud_detail").hidden = st.engine !== "cloud";
  $<HTMLElement>("ob_engine_hint").textContent = "";
  showCounter();
}

async function testServer(): Promise<void> {
  const form = devStore.form({ host: $<HTMLInputElement>("ob_host").value.trim(), token: $<HTMLInputElement>("ob_token").value.trim() });
  status("ob_server_status", "Connecting…");
  try {
    const h = await invoke<Health>("dev_test_connection", { form });
    if (h.ok) status("ob_server_status", `Connected${h.server ? ` to ${h.server}` : ""}${h.rtt_ms ? ` · ${h.rtt_ms.median} ms` : ""}.`, true);
    else status("ob_server_status", h.message ?? h.error ?? "No answer.", false);
  } catch (e) {
    status("ob_server_status", errText(e), false);
  }
}

/** Save the server fields (address + token) with `patch`; errors go next to them. */
async function saveServer(patch: Partial<DevForm>): Promise<boolean> {
  $<HTMLElement>("ob_host_err").textContent = "";
  $<HTMLElement>("ob_token_err").textContent = "";
  const host = $<HTMLInputElement>("ob_host").value.trim();
  if (!host) {
    st.serverOpen = true;
    renderServerFold(st.step === "engine");
    $<HTMLElement>("ob_host_err").textContent = "Type your server's address.";
    return false;
  }
  try {
    const token = $<HTMLInputElement>("ob_token").value.trim();
    await devStore.save({ ...patch, host, token });
    if (token) st.tokenSaves += 1;
    $<HTMLInputElement>("ob_token").value = "";
    return true;
  } catch (e) {
    if (Array.isArray(e)) {
      for (const f of e as FieldError[]) {
        const slot = document.getElementById(`ob_${f.field}_err`);
        if (slot) slot.textContent = f.message;
        else status("ob_server_status", f.message, false);
      }
    } else status("ob_server_status", String(e), false);
    return false;
  }
}

/** "Connect" on step 4: save the server (downloads only; speech keeps its route), then test it. */
async function connectServer(): Promise<void> {
  if (await saveServer({})) await testServer();
}

/** Step 2's server fields live on step 2, or on step 4 while "Your server" is picked with none saved. */
function placeServerForm(): void {
  const form = $<HTMLElement>("ob_server_detail");
  const here = st.step === "cleanup" ? st.step : "engine";
  const slot = document.querySelector<HTMLElement>(`#onboarding [data-ob-server-slot="${here}"]`);
  if (slot && form.parentElement !== slot) slot.append(form);
  form.hidden = here === "engine" ? st.engine !== "server" : !srcView().serverForm;
  renderServerFold(here === "engine");
  $<HTMLButtonElement>("ob_test_conn").textContent = here === "engine" ? "Test" : "Connect";
  $<HTMLElement>("ob_server_help").textContent =
    here === "engine" ? "No server yet? Pick This PC above, or" : "No server yet? Download from Hugging Face, or";
}

/**
 * "I already have a server ▸": on step 2 the address and token sit behind it
 * (open when one is saved); on step 4 they are what the form is for.
 */
function renderServerFold(onEngineStep: boolean): void {
  const more = $<HTMLButtonElement>("ob_server_more");
  const open = !onEngineStep || st.serverOpen || hasServer();
  more.hidden = !onEngineStep || hasServer();
  more.setAttribute("aria-expanded", String(open));
  (more.querySelector(".lw-ob-chev") as HTMLElement).textContent = open ? "▾" : "▸";
  $<HTMLElement>("ob_server_fields").hidden = !open;
}

/** Continue on the engine step: turn the picked engine on and put it first. */
async function applyEngine(): Promise<boolean> {
  const dev = views.dev;
  if (!dev) return true;
  if (engineKept()) return true;
  const order: Route[] = [ENGINE_ROUTE[st.engine], ...dev.form.route_order.filter((r) => r !== ENGINE_ROUTE[st.engine])];
  if (st.engine === "pc") {
    // An unset server would only add a "not set up" warning.
    await devStore.save({ allow_client: true, allow_server: dev.form.allow_server && !!dev.form.host, route_order: order });
    return true;
  }
  if (st.engine === "server") return saveServer({ allow_server: true, route_order: order });
  // OpenRouter speech: key + this use's consent (D18/D28), then cloud first.
  if (!(await saveKey("ob_stt_key", "ob_cloud_status"))) return false;
  if (!$<HTMLInputElement>("ob_stt_consent").checked) {
    status("ob_cloud_status", "Tick “Send my audio to OpenRouter” to use it.", false);
    return false;
  }
  const s = views.or?.stt;
  try {
    const v = await invoke<OrView>("dev_or_set_stt", {
      form: { allow: true, consent: true, language: s?.language ?? "", send_dictionary: s?.send_dictionary ?? true, stt_path: s?.stt_path ?? "" },
    });
    views.setOr(v);
    await refreshDev();
    if (views.dev && views.dev.form.route_order[0] !== "cloud") await devStore.save({ route_order: order });
    status("ob_cloud_status", "");
    return true;
  } catch (e) {
    status("ob_cloud_status", errText(e), false);
    return false;
  }
}

// ------------------------------------------------------------ step: model

function activeModel(): SpeechRow {
  return prefsStore.get()?.asr_engine === "parakeet" ? "parakeet" : "whisper";
}

/** The Whisper row: the model the prefs' file stands for. */
function whisper(): WhisperRow {
  return whisperRow(prefsStore.get()?.model_path, models.catalog());
}

/** The catalogue model behind a row (the Whisper row follows the prefs' file). */
function rowEntry(id: SpeechRow, w: WhisperRow = whisper()): CatalogEntry | undefined {
  if (id === "parakeet") return models.entry(PARAKEET_ID);
  return w.id ? models.entry(w.id) : undefined;
}

/**
 * Whether the row's model is on this PC: the one in use as the backend
 * found its file, else the catalogue's `installed`. A file nothing knows
 * about (the backend said nothing, no catalogue entry) is "unknown": never
 * shown Installed or saved on a guess.
 */
function rowFile(id: SpeechRow, e: CatalogEntry | undefined): FileState {
  const known = id === activeModel() ? fileState(st.model) : "unknown";
  if (known !== "unknown") return known;
  return e ? (e.installed ? "present" : "missing") : "unknown";
}

/** Percent of a running download (or check) of `e`, else null. */
function downloadPct(e: CatalogEntry | undefined): number | null {
  if (!e || !models.busy(e.id)) return null;
  const p = models.progress(e.id);
  return p?.total ? Math.round((p.received / p.total) * 100) : 0;
}

function pickModelRow(id: SpeechRow): void {
  st.modelPick = id;
  renderModel();
}

function renderModel(): void {
  const w = whisper();
  const other = $<HTMLElement>("ob_model_other");
  const more = $<HTMLButtonElement>("ob_model_more");
  // The Whisper row picked (a re-run with Whisper in use) keeps "More languages" open.
  if (st.modelPick === "whisper") st.moreOpen = true;
  other.hidden = !st.moreOpen;
  more.setAttribute("aria-expanded", String(st.moreOpen));
  (more.querySelector(".lw-ob-chev") as HTMLElement).textContent = st.moreOpen ? "▾" : "▸";
  let picked = { installed: false, busy: false, canDownload: false, why: "" };
  for (const r of document.querySelectorAll<HTMLElement>("#onboarding [data-ob-model]")) {
    const id = r.dataset.obModel as SpeechRow;
    const e = rowEntry(id, w);
    const on = id === st.modelPick;
    r.classList.toggle("is-on", on);
    r.setAttribute("aria-checked", String(on));
    (r.querySelector("[data-ob-rec]") as HTMLElement).hidden = id !== "parakeet";
    (r.querySelector("[data-ob-model-title]") as HTMLElement).textContent = id === "parakeet" ? PARAKEET_TITLE : w.title;
    const size = e ? `${mb(e.size)} · ` : "";
    (r.querySelector("[data-ob-model-desc]") as HTMLElement).textContent =
      id === "parakeet" ? `${size}English only, the quickest` : `${size}Many languages${e ? "" : " · your own file"}`;
    const pct = downloadPct(e);
    const file = rowFile(id, e);
    const installed = file === "present";
    const c = chip(file, pct);
    const badge = r.querySelector<HTMLElement>("[data-ob-chip]") as HTMLElement;
    badge.hidden = !c;
    badge.textContent = c?.text ?? "";
    badge.className = `fl-badge fl-badge-${c?.kind ?? "light"}`;
    badge.dataset.state = c?.state ?? "unknown";
    const busy = pct !== null;
    const canStart = !!e && models.canStart(e);
    const dl = r.querySelector<HTMLButtonElement>("[data-ob-dl]") as HTMLButtonElement;
    dl.hidden = installed || busy || !canStart;
    dl.disabled = models.busy();
    dl.textContent = e && models.progress(e.id)?.state === "error" ? "Try again" : "Download";
    (r.querySelector("[data-ob-dl-cancel]") as HTMLElement).hidden = !busy;
    renderProgress(
      r.querySelector("[data-ob-dl-box]") as HTMLElement,
      r.querySelector("[data-ob-dl-bar]") as HTMLElement,
      r.querySelector("[data-ob-dl-line]") as HTMLElement,
      e,
      busy,
    );
    if (on) {
      const why =
        installed || busy || canStart
          ? ""
          : e && models.notOnServer(e)
            ? "Your server doesn't have this model. After setup, download it from the internet in Settings → Speech to text."
            : "This version of Murmur can't download it. Add the file after setup in Settings → Speech to text.";
      picked = { installed, busy, canDownload: canStart, why };
    }
  }
  const m = st.model;
  const failed = m?.state === "failed" && st.modelPick === activeModel();
  status("ob_model_status", failed ? `It didn't load: ${m?.message ?? ""}` : picked.why, failed ? false : undefined);
  // Continue once the picked model is there or downloading. One this setup
  // can't fetch is added in Settings, so setup never gets stuck here.
  const next = nextBtn("model");
  next.disabled = !picked.installed && !picked.busy && picked.canDownload;
  $<HTMLElement>("ob_model_hint").textContent = next.disabled
    ? "Download it to continue"
    : picked.busy
      ? "You can carry on. It finishes in the background."
      : "";
}

/** Continue on the model step: the picked model becomes the one in use (a download sets it when done). */
async function applyModel(): Promise<boolean> {
  const pick = st.modelPick;
  if (pick === activeModel() || rowFile(pick, rowEntry(pick)) !== "present") return true;
  try {
    await prefsStore.save({ asr_engine: pick });
    st.model = (await invoke<ModelInfo>("get_model_status")).status;
    return true;
  } catch (e) {
    status("ob_model_status", errText(e), false);
    return false;
  }
}

/** "Download from" over where downloads come from (the backend's catalogue). */
function srcView(): SourceView {
  return sourceView({ pick: st.pick, effective: downloadSource(), remote: !!views.dev?.caps.remote, serverSet: hasServer() });
}

/** Step 4's "Download from: Your server | Hugging Face" (D79) and the server fields under it. */
function renderSource(dl: boolean): SourceView {
  const v = srcView();
  const box = panel("cleanup").querySelector<HTMLElement>("[data-ob-src-box]") as HTMLElement;
  box.hidden = !dl && !v.serverButton;
  for (const b of box.querySelectorAll<HTMLButtonElement>("[data-ob-src]")) {
    const on = b.dataset.obSrc === v.shown;
    b.hidden = b.dataset.obSrc === "server" && !v.serverButton;
    b.classList.toggle("is-on", on);
    b.setAttribute("aria-checked", String(on));
  }
  (box.querySelector("[data-ob-src-hint]") as HTMLElement).textContent = sourceHint(v, views.dev?.form.host.trim() ?? "");
  placeServerForm();
  return v;
}

/** A download's progress bar and line (3b), shared by the speech and cleanup rows. */
function renderProgress(box: HTMLElement, bar: HTMLElement, line: HTMLElement, e: CatalogEntry | undefined, busy: boolean): void {
  const p = e ? models.progress(e.id) : undefined;
  const shown = !!p && p.state !== "done";
  box.hidden = !shown;
  box.classList.toggle("is-err", p?.state === "error");
  bar.hidden = !busy;
  if (!p || !shown) return;
  const pct = p.total ? Math.round((p.received / p.total) * 100) : 0;
  bar.setAttribute("aria-valuenow", String(pct));
  (bar.firstElementChild as HTMLElement).style.width = `${pct}%`;
  line.textContent = progressLine(p, e);
}

/** "Choose file" for the cleanup model: the native picker fills the field. */
async function pickLlm(): Promise<void> {
  try {
    const p = await pickPath("file", "Choose an AI cleanup model", [{ name: "GGUF model", pattern: "*.gguf" }, { name: "All files", pattern: "*.*" }]);
    if (p) $<HTMLInputElement>("ob_llm_path").value = p;
  } catch (e) {
    console.error("pick_path", e);
  }
}

// ------------------------------------------------------------ step: cleanup

function pickCleanup(c: SetupCleanup): void {
  if (!hasCleanup(c)) return;
  st.cleanup = c;
  st.cleanupPicked = true;
  renderCleanup();
  // The consent (and what's missing) sits under the options: bring it into
  // view, or at 1040 x 675 it stays below the footer unseen.
  if (c === "cloud") $<HTMLElement>("ob_or_detail").scrollIntoView({ block: "nearest" });
}

function llmLine(s: LlmStatus | undefined): string {
  switch (s?.state) {
    case "ready":
      return `Ready: ${s.path ?? ""}`;
    case "loading":
      return `Loading ${s.path ?? ""}…`;
    case "missing":
      return `Not found: ${s.path ?? ""}. Put the file there or type where it is.`;
    case "plugin_missing":
      return `The cleanup engine is missing next to the app: ${s.message ?? ""}`;
    case "failed":
      return `Could not load it: ${s.message ?? ""}`;
    default:
      return "Empty = the default file in your models folder.";
  }
}

/** The options: selected, Recommended, and "Your server" shown only with a server engine, disabled with
 *  the reason until a server is saved (step 2). Redrawn when the server or its cleanup status changes. */
function renderCleanupOptions(): void {
  const facts = serverFacts();
  const server = serverOption(facts);
  const rec = recommendedCleanup(facts, hasCleanup("pc"));
  for (const b of document.querySelectorAll<HTMLButtonElement>("#onboarding [data-ob-cleanup]")) {
    const id = b.dataset.obCleanup as SetupCleanup;
    const on = id === st.cleanup;
    b.setAttribute("aria-checked", String(on));
    b.classList.toggle("is-on", on);
    b.disabled = !hasCleanup(id);
    // "Not in this build" is for a missing engine; the server option says why instead.
    (b.querySelector("[data-ob-na]") as HTMLElement).hidden = id === "server" || hasCleanup(id);
    (b.querySelector("[data-ob-rec]") as HTMLElement).hidden = id !== rec || !hasCleanup(id);
    if (id === "cloud") {
      // D96: named by the saved provider.
      const copy = setupCloudCopy(providerOf(views.or));
      (b.querySelector("[data-ob-title]") as HTMLElement).textContent = copy.cleanTitle;
      (b.querySelector("[data-ob-desc]") as HTMLElement).textContent = copy.cleanDesc;
    }
    if (id !== "server") continue;
    b.hidden = server.hidden;
    const reason = b.querySelector("[data-ob-reason]") as HTMLElement;
    reason.textContent = server.reason;
    reason.hidden = !server.reason;
    (b.querySelector("[data-ob-desc]") as HTMLElement).hidden = !!server.reason;
  }
}

function renderCleanup(): void {
  renderCleanupOptions();
  $<HTMLElement>("ob_sample").textContent = SAMPLE[st.cleanup];
  $<HTMLElement>("ob_llm_detail").hidden = st.cleanup !== "pc";
  $<HTMLElement>("ob_or_detail").hidden = st.cleanup !== "cloud";
  renderCloudConsent();
  status("ob_cleanup_status", "");
  renderLlmModel();
}

/** The cleanup model's file field, its status and the Download row (redrawn as downloads and prefs change). */
function renderLlmModel(): void {
  const p = prefsStore.get();
  const path = $<HTMLInputElement>("ob_llm_path");
  if (p && document.activeElement !== path) path.value = p.llm_model_path ?? "";
  if (st.info) path.placeholder = st.info.default_llm_path;
  $<HTMLElement>("ob_llm_status").textContent = llmLine(st.info?.llm);
  renderLlmDownload();
}

/** The cleanup model setup recommends (the CPU pick: setup doesn't probe the graphics chip). */
function llmEntry(): CatalogEntry | undefined {
  return recommendedLlm(models.catalog().filter((c) => c.engine === "llm"), false);
}

/** Step 4's "Download from" and Download row for the cleanup model (D79), as on step 3. */
function renderLlmDownload(): void {
  const e = llmEntry();
  const v = renderSource(!!e?.available);
  $<HTMLElement>("ob_llm_hf").hidden = !v.hfLinks;
  const row = $<HTMLElement>("ob_llm_dl");
  row.hidden = !e;
  if (!e) return;
  const busy = models.busy(e.id);
  const missing = models.notOnServer(e) && !e.installed;
  $<HTMLElement>("ob_llm_dl_title").textContent = e.title;
  // Setup's own words: the catalogue's description talks about GPUs.
  $<HTMLElement>("ob_llm_dl_desc").textContent = missing
    ? `${mb(e.size)}. Your server doesn't have this one. Ask whoever runs it to add it, or switch to Hugging Face.`
    : `Works on any PC · ${mb(e.size)}${e.installed ? " · downloaded and checked" : ""}`;
  $<HTMLElement>("ob_llm_missing").hidden = !missing || busy;
  const start = $<HTMLButtonElement>("ob_llm_dl_start");
  start.hidden = !v.downloads || !models.canStart(e) || e.installed || busy;
  start.disabled = models.busy();
  start.textContent = models.progress(e.id)?.state === "error" ? "Try again" : "Download";
  $<HTMLButtonElement>("ob_llm_dl_cancel").hidden = !busy;
  renderProgress($<HTMLElement>("ob_llm_dl_box"), $<HTMLElement>("ob_llm_dl_bar"), $<HTMLElement>("ob_llm_dl_line"), e, busy);
}

/** Re-draw the open step after the server, the catalogue or a download changed. */
function rerender(): void {
  if (st.step === "try") renderSummary();
  if (st.step === "model") renderModel();
  else if (st.step === "engine") placeServerForm();
  else if (st.step === "cleanup") {
    renderCleanupOptions();
    renderCloudConsent();
    renderLlmModel();
  }
}

/**
 * The Cloud option's consent, for the provider saved in Settings → AI provider
 * (D78): setup asks only for consent; the provider, key and address are set
 * there. Consent is bound to the destination's host (D28/D52), so it names it.
 */
function renderCloudConsent(): void {
  const c = views.or?.cleanup;
  const custom = c?.provider === "custom";
  const host = custom ? urlHost(c?.custom.base_url ?? "") : "";
  $<HTMLElement>("ob_clean_consent_title").textContent = custom
    ? "Send my dictated text to my own AI server"
    : "Send my dictated text to OpenRouter";
  $<HTMLElement>("ob_clean_consent_desc").textContent = custom
    ? `Your words go to ${host || "the address you set in Settings → AI provider"} as text. A new address asks again.`
    : "Your words go to openrouter.ai as text, to zero-retention providers only. Your voice is not sent unless Cloud speech to text is also on.";
  // Shown with the option, before Continue moves on: Cloud would be on but
  // skipped until then, and the next cleaner would run instead.
  const missing = $<HTMLElement>("ob_cloud_missing");
  const line = cloudMissing();
  missing.textContent = line ?? "";
  missing.hidden = !line;
}

/** What setup's Cloud option would still need before it cleans (null = nothing). */
function cloudMissing(): string | null {
  const c = views.or?.cleanup;
  return cloudMissingLine({
    provider: c?.provider === "custom" ? "custom" : "openrouter",
    keySet: keySet(),
    baseUrl: c?.custom.base_url ?? "",
    model: c?.custom.model ?? "",
  });
}

function urlHost(url: string): string {
  try {
    return new URL(url.trim()).host;
  } catch {
    return "";
  }
}

/** The saved consent for the saved Cloud provider. */
function cloudConsent(): boolean {
  const c = views.or?.cleanup;
  if (!c) return false;
  return c.provider === "custom" ? c.custom.consent : c.consent;
}

function cleanupForm(allow: boolean, consent: boolean): Record<string, unknown> {
  const c = views.or?.cleanup;
  return {
    allow,
    consent,
    // Choosing Cloud here makes it the first cleaner (D36); otherwise
    // the cleanup order is kept (undefined = keep).
    order: allow ? ["cloud", "local", "server"] : undefined,
    sort: c?.sort ?? "latency",
    timeout_ms: c?.timeout_ms ?? 2500,
    send_surrounding_text: c?.send_surrounding_text ?? false,
    send_screen_words: c?.send_screen_words ?? false,
    send_app_name: c?.send_app_name ?? false,
  };
}

async function applyCleanup(): Promise<boolean> {
  const p = prefsStore.get();
  if (!p) return true;
  if (cleanupKept()) return true;
  const level = p.cleanup_level === "none" ? "light" : p.cleanup_level;
  // Read before the saves below: each one redraws the step from the saved prefs.
  const llmPath = $<HTMLInputElement>("ob_llm_path").value.trim() || null;
  try {
    if (st.cleanup === "cloud") {
      if (!$<HTMLInputElement>("ob_clean_consent").checked) {
        status("ob_cleanup_status", `Tick “${$<HTMLElement>("ob_clean_consent_title").textContent ?? ""}” to use it.`, false);
        return false;
      }
      // The saved provider; a missing key or address is added in Settings → AI provider
      // (until then Cloud is on but skipped, and the next cleaner runs).
      views.setOr(await invoke<OrView>("dev_or_set_cleanup", { form: cleanupForm(true, true) }));
      await prefsStore.save({ cleanup_level: level, spoken_cleanup: true, raw_mode: false });
      return true;
    }
    // Rules, this PC or your server: Cloud cleanup off if it was on.
    if (views.or?.supported && views.or.cleanup.allow) {
      views.setOr(await invoke<OrView>("dev_or_set_cleanup", { form: cleanupForm(false, false) }));
    }
    // Your server / this PC first in the D36 cleanup order, through the same
    // command as Settings → AI cleanup's switches (features/ai.ts).
    const order = enginesFor(st.cleanup, cleanupEngines());
    if (order) views.setOr(await invoke<OrView>("dev_cleanup_set_engines", { engines: order }));
    if (st.cleanup === "server") {
      // This PC's switch stays as it was: a backup when it is on.
      await prefsStore.save({ cleanup_level: level, spoken_cleanup: true, raw_mode: false });
    } else if (st.cleanup === "rules") {
      await prefsStore.save({ llm_enabled: false, cleanup_level: "none", spoken_cleanup: true, raw_mode: false });
    } else {
      await prefsStore.save({
        llm_enabled: true,
        cleanup_level: level,
        spoken_cleanup: true,
        raw_mode: false,
        llm_model_path: llmPath,
      });
    }
    return true;
  } catch (e) {
    status("ob_cleanup_status", errText(e), false);
    return false;
  }
}

// ------------------------------------------------------------ step: try

let recordingPtt = false;

function showRecording(on: boolean): void {
  recordingPtt = on;
  $<HTMLElement>("ob_ptt_rec").hidden = !on;
  $<HTMLElement>("ob_ptt_type").hidden = !on;
  $<HTMLButtonElement>("ob_ptt_change").textContent = on ? "Cancel" : "Change";
}

/** Change: hold the new push-to-talk keys and let go (keyboard hook,
 *  lib/keycapture.ts); it saves through the field's `change`. */
async function recordPtt(): Promise<void> {
  if (recordingPtt) {
    showRecording(false);
    stopCapture();
    return;
  }
  showRecording(true);
  const r = await captureChord();
  if (!recordingPtt) return; // Cancel / Type instead
  showRecording(false);
  if ("chord" in r) {
    const i = $<HTMLInputElement>("ob_ptt");
    i.value = r.chord;
    i.dispatchEvent(new Event("change"));
  } else if (r.message) {
    $<HTMLElement>("ob_shortcut_desc").textContent = r.message;
  }
}

/** Type instead: the text field, for keys that can't be pressed. */
function typePtt(): void {
  showRecording(false);
  stopCapture();
  const i = $<HTMLInputElement>("ob_ptt");
  i.hidden = false;
  $<HTMLButtonElement>("ob_ptt_change").hidden = true;
  i.focus();
  i.select();
}

function renderTry(): void {
  const p = prefsStore.get();
  const ptt = p?.shortcuts.ptt || "Right Alt";
  $<HTMLElement>("ob_keys").replaceChildren(...ptt.split("+").map((k) => el("kbd", "fl-kbd", k.trim())));
  const hf = p?.shortcuts.hands_free;
  $<HTMLElement>("ob_shortcut_desc").textContent = `Hold ${ptt.replace(/\+/g, " + ")} to talk.${hf ? ` ${hf.replace(/\+/g, " + ")} = hands-free.` : ""}`;
  const pttInput = $<HTMLInputElement>("ob_ptt");
  if (document.activeElement !== pttInput) pttInput.value = ptt;
  const mic = $<HTMLSelectElement>("ob_mic");
  mic.value = p?.microphone ?? "";
}

const WHERE: Record<string, string> = {
  local: "on this PC",
  client: "on this PC",
  remote: "by your server",
  server: "by your server",
};

/** Where the try-it dictation ran; the cloud route by the saved provider (D96). */
const whereRan = (engine: string): string | undefined => (engine === "cloud" ? cloudWhere(providerOf(views.or)) : WHERE[engine]);

function setResult(line: TryLine): void {
  const r = $<HTMLElement>("ob_result");
  r.hidden = false;
  r.classList.toggle("is-bad", line.bad);
  $<HTMLElement>("ob_result_text").textContent = line.text;
}

function tryOpen(): boolean {
  return st.step === "try" && !$<HTMLElement>("onboarding").hidden;
}

function showResult(v: RouteView): void {
  const last = v.last;
  if (!last || last.at_ms < st.openedAt || !tryOpen()) return;
  st.resultAt = Date.now();
  const good = last.status === "FORMATTED" || last.status === "RAW_TRANSCRIPT";
  const where = whereRan(last.engine ?? "") ?? (last.engine ? `by ${last.engine}` : "");
  const secs = (ms: number): string => `${(ms / 1000).toFixed(1)} s`;
  pressLine({
    bad: !good,
    text: good
      ? `It works. Heard ${where} in ${secs(last.asr_ms)}${last.total_ms > last.asr_ms ? `, ready in ${secs(last.total_ms)}` : ""}.`
      : `That didn't work: ${last.notice ?? last.empty_reason ?? last.status}. Try again, or check Settings → Speech to text after setup.`,
  });
}

let recheckTimer: ReturnType<typeof setTimeout> | undefined;

/** D75: say up front when a press cannot work (no model, no server) instead
 *  of leaving the step silent. A press that already reached the pill wins.
 *  While the line is shown it is checked again every RECHECK_MS, so a server
 *  that comes up or a model that finishes clears it. */
async function checkReady(): Promise<void> {
  clearTimeout(recheckTimer);
  const opened = st.openedAt;
  let line: TryLine | null;
  try {
    line = readinessLine(await invoke<Readiness>("setup_readiness"), models.busy());
  } catch (e) {
    line = readinessErrorLine(e);
  }
  if (!tryOpen() || st.openedAt !== opened || st.line === "press") return;
  if (line) {
    setResult(line);
    st.line = "check";
    recheckTimer = setTimeout(recheckReady, RECHECK_MS);
  } else {
    if (st.line === "check") $<HTMLElement>("ob_result").hidden = true; // ready now
    st.line = "none";
  }
}

/** Check again while the "can't work yet" line is up (timer, model events). */
function recheckReady(): void {
  if (tryOpen() && st.line === "check") void checkReady();
}

/** A press reached the pill or a dictation finished: it owns the line now. */
function pressLine(line: TryLine): void {
  clearTimeout(recheckTimer);
  st.line = "press";
  setResult(line);
}

// ------------------------------------------------------------ navigation

/** "Step n of N" on the open step (N follows step 2's pick: no model step off this PC). */
function showCounter(): void {
  const list = steps();
  const over = panel(st.step).querySelector<HTMLElement>(".lw-ob-head .fl-overline");
  if (over) over.textContent = `Step ${list.indexOf(st.step) + 1} of ${list.length}`;
}

function show(s: Step): void {
  st.step = s;
  for (const p of document.querySelectorAll<HTMLElement>("#onboarding [data-ob-step]")) p.hidden = p.dataset.obStep !== s;
  const scroller = panel(s).querySelector<HTMLElement>("[data-ob-scroll]");
  if (scroller) scroller.scrollTop = 0;
  if (s === "engine") renderEngine();
  else showCounter();
  if (s === "model") {
    renderModel();
    // A fresh listing for "Not on your server" each time the step opens.
    if (models.fromServer()) void models.refreshServer();
  }
  if (s === "cleanup") {
    // A server saved in step 2 on this run counts; a pick made here stays.
    if (!st.cleanupPicked || !hasCleanup(st.cleanup)) st.cleanup = savedCleanup();
    renderCleanup();
    if (models.fromServer()) void models.refreshServer();
    // Whether the server has an AI model (D36), for "Recommended"; the answer redraws through views.onOr.
    void pollCleanupStatus();
  }
  if (s === "try") {
    renderTry();
    st.openedAt = Date.now();
    st.line = "none";
    $<HTMLElement>("ob_result").hidden = true;
    void checkReady();
    renderSummary();
    $<HTMLTextAreaElement>("ob_try_box").focus();
  }
  panel(s).querySelector<HTMLElement>('[data-ob="next"]')?.focus({ preventScroll: true });
  if (s === "try") $<HTMLTextAreaElement>("ob_try_box").focus();
}

async function next(): Promise<void> {
  if (st.busy) return;
  st.busy = true;
  const btn = nextBtn(st.step);
  btn.disabled = true;
  try {
    let ok = true;
    if (st.step === "engine") ok = await applyEngine().catch((e) => {
      $<HTMLElement>("ob_engine_hint").textContent = errText(e);
      return false;
    });
    if (st.step === "model") ok = await applyModel();
    if (st.step === "cleanup") ok = await applyCleanup();
    if (!ok) return;
    if (st.step === "try") {
      await finish();
      return;
    }
    const list = steps();
    show(list[list.indexOf(st.step) + 1]);
  } finally {
    st.busy = false;
    btn.disabled = false;
    if (st.step === "model") renderModel();
  }
}

/** "Download from": the pick shows at once; the backend switches after the save. */
function pickSource(src: ModelSource): void {
  st.pick = src;
  status("ob_server_status", "");
  rerender();
  void setDownloadSource(src).catch((err) => console.error("model_source", err));
}

function back(): void {
  const list = steps();
  const i = list.indexOf(st.step);
  if (i > 0) show(list[i - 1]);
}

async function finish(): Promise<void> {
  try {
    await prefsStore.save({ onboarded: true });
  } catch (e) {
    console.error("onboarded save failed", e);
  }
  $<HTMLElement>("onboarding").hidden = true;
  document.body.classList.remove("lw-ob-open");
  onDone();
}

// ------------------------------------------------------------ re-run: what Finish changes (D95)

/** The engine the saved settings would use first (an address counts trimmed, as everywhere). */
function savedSpeech(): Engine | null {
  const dev = views.dev?.form;
  if (!dev) return null;
  return savedEngine({ route_order: dev.route_order, allow_client: dev.allow_client, allow_server: dev.allow_server, host: dev.host, cloudStt: !!views.or?.stt.allow });
}

/** Names for the line only; choices are compared as saved. */
const LABELS: Labels = {
  modelName: (m: SpeechModel) => (m.asr_engine === "parakeet" ? PARAKEET_TITLE : whisperRow(m.model_path, models.catalog()).title),
  get provider() {
    return providerOf(views.or);
  },
};

/** Setup's choices as the saved settings have them now. */
function savedConfig(): SetupConfig {
  const p = prefsStore.get();
  return {
    engine: savedSpeech(),
    host: views.dev?.form.host.trim() ?? "",
    tokenSaves: st.tokenSaves,
    model: has("pc") && p ? { asr_engine: p.asr_engine, model_path: p.model_path, parakeet_model_dir: p.parakeet_model_dir } : null,
    source: p?.model_source ?? "",
    cleanup: savedCleanup(),
    llmPath: p?.llm_model_path ?? null,
    ptt: p?.shortcuts.ptt || "Right Alt",
    mic: p?.microphone ?? null,
  };
}

async function snapshot(): Promise<Before> {
  const dev = views.dev?.form;
  const or = views.or;
  const p = prefsStore.get();
  // The saved server (address and token) is kept by the backend, so Keep can put it back without the token ever reaching this page.
  if (has("server")) await invoke("setup_server_snapshot").catch((e) => console.error("setup_server_snapshot", e));
  return {
    cfg: savedConfig(),
    dev: { allow_client: !!dev?.allow_client, allow_server: !!dev?.allow_server, route_order: [...(dev?.route_order ?? [])] },
    stt: or?.supported ? { ...or.stt } : null,
    cleanup: or?.supported ? { allow: or.cleanup.allow, consent: cloudConsent(), engines: cleanupEngines().map((e) => ({ ...e })) } : null,
    prefs: p
      ? {
          asr_engine: p.asr_engine,
          model_path: p.model_path,
          parakeet_model_dir: p.parakeet_model_dir,
          model_source: p.model_source,
          llm_enabled: p.llm_enabled,
          cleanup_level: p.cleanup_level,
          spoken_cleanup: p.spoken_cleanup,
          raw_mode: p.raw_mode,
          llm_model_path: p.llm_model_path,
          shortcuts: { ...p.shortcuts },
          microphone: p.microphone,
        }
      : {},
  };
}

/** Step 2 on a re-run with the saved engine still picked and nothing new typed: Continue saves nothing. */
function engineKept(): boolean {
  const b = st.rerunSnapshot;
  if (!b || st.engine !== b.cfg.engine) return false;
  if (st.engine === "server") {
    return $<HTMLInputElement>("ob_host").value.trim() === (views.dev?.form.host.trim() ?? "") && !$<HTMLInputElement>("ob_token").value.trim();
  }
  if (st.engine === "cloud") return !$<HTMLInputElement>("ob_stt_key").value.trim() && $<HTMLInputElement>("ob_stt_consent").checked;
  return true;
}

/** Step 4 on a re-run with the saved option still picked (and, on this PC, the same model file): Continue saves nothing. */
function cleanupKept(): boolean {
  const b = st.rerunSnapshot;
  if (!b || st.cleanup !== b.cfg.cleanup) return false;
  if (st.cleanup === "pc") return ($<HTMLInputElement>("ob_llm_path").value.trim() || null) === (prefsStore.get()?.llm_model_path ?? null);
  if (st.cleanup === "cloud") return $<HTMLInputElement>("ob_clean_consent").checked;
  return true;
}

function changes(): SetupChange[] {
  return st.rerunSnapshot ? setupChanges(st.rerunSnapshot.cfg, savedConfig(), LABELS) : [];
}

/** Step 5 on a re-run: the line saying what Finish leaves changed, with Keep / Change. */
function renderSummary(): void {
  const box = $<HTMLElement>("ob_summary");
  box.hidden = !st.rerunSnapshot;
  if (!st.rerunSnapshot) return;
  const list = changes();
  $<HTMLElement>("ob_summary_text").textContent = changesLine(list);
  $<HTMLElement>("ob_summary_keep").hidden = !list.length;
  $<HTMLElement>("ob_summary_change").hidden = !list.length;
}

/** Keep: put back what each changed choice had when setup opened. */
async function keepBefore(): Promise<void> {
  const b = st.rerunSnapshot;
  if (!b) return;
  const keys = new Set(changes().map((c) => c.key));
  const savedPrefs = <K extends keyof Prefs>(...ks: K[]): Partial<Prefs> =>
    Object.fromEntries(ks.filter((k) => k in b.prefs).map((k) => [k, b.prefs[k]])) as Partial<Prefs>;
  try {
    // The server first: the route order below may name it.
    if (keys.has("host") || keys.has("token")) {
      views.setDev(await invoke<DevView>("setup_server_restore"));
      st.tokenSaves = 0;
      $<HTMLInputElement>("ob_host").value = views.dev?.form.host ?? "";
    }
    if (keys.has("engine")) {
      if (b.stt && views.or && views.or.stt.allow !== b.stt.allow) {
        const s = b.stt;
        views.setOr(
          await invoke<OrView>("dev_or_set_stt", {
            form: { allow: s.allow, consent: s.consent, language: s.language, send_dictionary: s.send_dictionary, stt_path: s.stt_path },
          }),
        );
      }
      await devStore.save(b.dev);
    }
    const back: Partial<Prefs> = {};
    if (keys.has("model")) Object.assign(back, savedPrefs("asr_engine", "model_path", "parakeet_model_dir"));
    if (keys.has("source")) Object.assign(back, savedPrefs("model_source"));
    if (keys.has("cleanup") || keys.has("llmPath")) {
      if (keys.has("cleanup") && b.cleanup && views.or) {
        if (views.or.cleanup.allow !== b.cleanup.allow) {
          views.setOr(await invoke<OrView>("dev_or_set_cleanup", { form: cleanupForm(b.cleanup.allow, b.cleanup.consent) }));
        }
        views.setOr(await invoke<OrView>("dev_cleanup_set_engines", { engines: b.cleanup.engines }));
      }
      Object.assign(back, savedPrefs("llm_enabled", "cleanup_level", "spoken_cleanup", "raw_mode", "llm_model_path"));
    }
    if (keys.has("ptt")) Object.assign(back, savedPrefs("shortcuts"));
    if (keys.has("mic")) Object.assign(back, savedPrefs("microphone"));
    if (Object.keys(back).length) await prefsStore.save(back);
  } catch (e) {
    $<HTMLElement>("ob_summary_text").textContent = errText(e);
    return;
  }
  st.engine = b.cfg.engine ?? st.engine;
  st.cleanup = b.cfg.cleanup;
  st.cleanupPicked = false;
  st.modelPick = activeModel();
  st.pick = null;
  renderTry();
  renderSummary();
}

/** Change: back to the step of the first choice that changed. */
function changeFirst(): void {
  const first = changes()[0];
  if (!first) return;
  const to = STEP_OF[first.key];
  show(steps().includes(to) ? to : "engine");
}

// ------------------------------------------------------------ choices from the saved settings

function initialChoices(): void {
  const dev = views.dev?.form;
  const or = views.or;
  const p = prefsStore.get();
  // Once per run: a server saved on this run (Test / Connect) doesn't move the badge.
  const facts = { has, serverSaved: hasServer() };
  st.recEngine = recommendedEngine(facts);
  st.engine = initialEngine(savedSpeech(), !!p?.onboarded, facts);
  st.cleanup = savedCleanup();
  st.cleanupPicked = false;
  st.modelPick = initialModel(activeModel(), !!p?.onboarded, fileState(st.model));
  st.moreOpen = false;
  $<HTMLInputElement>("ob_host").value = dev?.host ?? "";
  $<HTMLInputElement>("ob_token").placeholder = dev?.token_saved ? "Saved" : "Paste the token";
  $<HTMLElement>("ob_token_desc").textContent = dev?.token_saved
    ? "A token is saved. Paste a new one only to replace it."
    : "Shown when you set up your server. Stored encrypted for your Windows account.";
  st.serverOpen = false;
  $<HTMLInputElement>("ob_stt_consent").checked = !!or?.stt.consent;
  $<HTMLInputElement>("ob_clean_consent").checked = cloudConsent();
  showKeyRows();
}

// ------------------------------------------------------------ public

export const onboarding = {
  /** Mount the overlay (hidden) once at boot. `done` runs after Finish / Set up later. */
  init(done: () => void): void {
    onDone = done;
    document.body.append(html(TEMPLATE));
    const root = $<HTMLElement>("onboarding");
    root.addEventListener("click", (e) => {
      const t = e.target as HTMLElement;
      const act = t.closest<HTMLElement>("[data-ob]")?.dataset.ob;
      if (act === "next") void next();
      else if (act === "back") back();
      else if (act === "skip") void finish();
      const eng = t.closest<HTMLElement>("[data-ob-engine]")?.dataset.obEngine;
      if (eng) pickEngine(eng as Engine);
      const cl = t.closest<HTMLElement>("[data-ob-cleanup]")?.dataset.obCleanup;
      if (cl) pickCleanup(cl as SetupCleanup);
      // A speech model row: a click anywhere on it picks it (Download too).
      const row = t.closest<HTMLElement>("[data-ob-model]")?.dataset.obModel;
      if (row && !t.closest("[data-ob-dl-cancel]")) pickModelRow(row as SpeechRow);
      if (t.closest("[data-ob-pick=llm]")) void pickLlm();
      const src = t.closest<HTMLElement>("[data-ob-src]")?.dataset.obSrc;
      if (src) pickSource(src as ModelSource);
      const dl = t.closest<HTMLElement>("[data-ob-dl]")?.dataset.obDl;
      const dlEntry = dl ? rowEntry(dl as SpeechRow) : undefined;
      if (dlEntry) void models.confirmAndStart(dlEntry.id);
      const stop = t.closest<HTMLElement>("[data-ob-dl-cancel]")?.dataset.obDlCancel;
      const stopEntry = stop ? rowEntry(stop as SpeechRow) : undefined;
      if (stopEntry) void models.cancel(stopEntry.id);
    });
    // Space / Enter on a focused speech model row picks it, as on a radio.
    for (const r of root.querySelectorAll<HTMLElement>("[data-ob-model]")) {
      r.addEventListener("keydown", (e) => {
        if (e.target !== r || (e.key !== " " && e.key !== "Enter")) return;
        e.preventDefault();
        pickModelRow(r.dataset.obModel as SpeechRow);
      });
    }
    $<HTMLButtonElement>("ob_model_more").addEventListener("click", () => {
      st.moreOpen = !st.moreOpen;
      // Folding it away while Whisper is picked goes back to the recommended one.
      if (!st.moreOpen && st.modelPick === "whisper") st.modelPick = "parakeet";
      renderModel();
    });
    $<HTMLInputElement>("ob_stt_consent").addEventListener("change", () => status("ob_cloud_status", ""));
    $<HTMLInputElement>("ob_clean_consent").addEventListener("change", () => status("ob_cleanup_status", ""));
    $<HTMLButtonElement>("ob_server_more").addEventListener("click", () => {
      st.serverOpen = !st.serverOpen;
      renderServerFold(true);
      if (st.serverOpen) $<HTMLInputElement>("ob_host").focus();
    });
    $<HTMLButtonElement>("ob_test_conn").addEventListener("click", () => void (st.step === "engine" ? testServer() : connectServer()));
    $<HTMLButtonElement>("ob_llm_dl_start").addEventListener("click", () => {
      const e = llmEntry();
      if (e) void models.confirmAndStart(e.id);
    });
    $<HTMLButtonElement>("ob_llm_dl_cancel").addEventListener("click", () => {
      const e = llmEntry();
      if (e) void models.cancel(e.id);
    });
    $<HTMLSelectElement>("ob_mic").addEventListener("change", () => {
      void prefsStore.save({ microphone: $<HTMLSelectElement>("ob_mic").value || null });
    });
    $<HTMLButtonElement>("ob_ptt_change").addEventListener("click", () => void recordPtt());
    $<HTMLButtonElement>("ob_summary_keep").addEventListener("click", () => void keepBefore());
    $<HTMLButtonElement>("ob_summary_change").addEventListener("click", changeFirst);
    $<HTMLButtonElement>("ob_ptt_type").addEventListener("click", typePtt);
    $<HTMLInputElement>("ob_ptt").addEventListener("change", async () => {
      const p = prefsStore.get();
      const v = $<HTMLInputElement>("ob_ptt").value.trim();
      if (!p || !v) return;
      try {
        await prefsStore.save({ shortcuts: { ...p.shortcuts, ptt: v } });
        $<HTMLInputElement>("ob_ptt").hidden = true;
        $<HTMLButtonElement>("ob_ptt_change").hidden = false;
      } catch (e) {
        $<HTMLElement>("ob_shortcut_desc").textContent = String(e);
        return;
      }
      renderTry();
    });
    // Download progress, and the prefs the backend saved when one finished.
    models.onChange(rerender);
    prefsStore.subscribe(rerender);
    views.onDev(rerender);
    views.onOr(rerender);
    void onEvent<{ id: string; state: string }>("model:download", (e) => {
      if (e.payload.state !== "done") return;
      void invoke<ModelInfo>("get_model_status").then((i) => {
        st.info = i;
        st.model = i.status;
        if (st.step === "model") renderModel();
        recheckReady();
      });
    });
    void onEvent<ModelStatus>("model:status", (e) => {
      st.model = e.payload;
      if (st.step === "model") renderModel();
      recheckReady();
    });
    void onEvent<LlmStatus>("llm:status", (e) => {
      if (st.info) st.info.llm = e.payload;
      if (st.step === "cleanup") $<HTMLElement>("ob_llm_status").textContent = llmLine(e.payload);
    });
    void onEvent<RouteView>("dev:route", (e) => showResult(e.payload));
    // D75: the pill's state, so a press shows here too: listening, working,
    // and why it failed (no model, server unreachable) instead of nothing.
    void onEvent<PillState>("dictation:status", (e) => {
      if (!tryOpen()) return;
      const line = pillLine(e.payload);
      if (line) pressLine(line);
    });
    void onEvent<null>("history:changed", () => {
      if (!tryOpen()) return;
      // Builds whose dictations bypass the router still say it arrived.
      // A route result just before or just after this one wins.
      const at = Date.now();
      setTimeout(() => {
        if (st.resultAt >= at - 2000 || !tryOpen()) return;
        st.resultAt = Date.now();
        pressLine({ text: "It works. Your dictation arrived.", bad: false });
      }, 300);
    });
  },

  isOpen(): boolean {
    return !$<HTMLElement>("onboarding").hidden;
  },

  /** Show setup from the first step, choices filled from the saved settings. */
  async open(): Promise<void> {
    try {
      const [info, mics] = await Promise.all([
        invoke<ModelInfo>("get_model_status"),
        invoke<string[]>("list_microphones"),
        models.refresh().catch(() => []),
      ]);
      st.info = info;
      st.model = info.status;
      $<HTMLSelectElement>("ob_mic").replaceChildren(new Option("System default", ""), ...mics.map((m) => new Option(m, m)));
    } catch (e) {
      console.error("setup: model status failed", e);
    }
    initialChoices();
    // A re-run keeps the current setup and says before Finish what it changes (D95).
    st.tokenSaves = 0;
    st.rerunSnapshot = prefsStore.get()?.onboarded ? await snapshot() : null;
    st.pick = null;
    $<HTMLElement>("onboarding").hidden = false;
    document.body.classList.add("lw-ob-open");
    show("welcome");
  },
};
