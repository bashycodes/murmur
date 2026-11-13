// First-run setup (docs/design/app-design/Onboard*.dc.html): a full-window
// overlay over the app shell, shown while prefs.onboarded is false or when
// the user asks for it again (Settings → General). Five steps: welcome,
// where speech runs, the speech model, AI cleanup, try it. Each step saves
// its choice with the existing commands when the user presses Continue
// (dev_set through the dev store, prefs through the prefs store,
// dev_cloud_set_key / dev_or_set_* for OpenRouter); Finish sets
// onboarded = true. Nothing downloads by itself (D9/D31): the model step
// points the app at files the user already has, or downloads a catalogue
// model after the user pressed Download and confirmed it (3a/3b, through
// features/models.ts), from the user's server or Hugging Face ("Download
// from", D79; picking Your server with none saved shows step 2's server
// fields right there). Step 4 offers the same for the cleanup model, and
// "Your server" (D36 server cleanup, lib/setup-cleanup.ts) once a server
// is saved. With
// speech on the server the step is optional ("Skip — use my server only").
// A server test or an OpenRouter key/consent happens only on the user's
// own click.

import { downloadSource, hasServer, mb, models, pickPath, progressLine, recommendedLlm, setDownloadSource, type CatalogEntry } from "../features/models.js";
import { devStore } from "../lib/dev-store.js";
import { $, el, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { captureChord, stopCapture } from "../lib/keycapture.js";
import { modelStepFoot, sourceHint, sourceView, type ModelSource, type SourceView } from "../lib/model-source.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { pillLine, readinessErrorLine, readinessLine, RECHECK_MS, type PillState, type Readiness, type TryLine, type TryLineSource } from "../lib/setup-try.js";
import type { CloudProvider, DevForm, DevView, FieldError, Health, KeyState, OrView, Route, RouteView } from "../lib/types.js";
import { initialEngine, recommendedEngine } from "../lib/setup-engine.js";
import { enginesFor, initialCleanup, recommendedCleanup, serverOption, type ServerFacts, type SetupCleanup } from "../lib/setup-cleanup.js";
import { views } from "../lib/views.js";
import { engines as cleanupEngines, pollCleanupStatus } from "../features/ai.js";
import { TEMPLATE } from "./templates.js";

type Engine = "pc" | "server" | "cloud";
type Step = "welcome" | "engine" | "model" | "cleanup" | "try";

interface ModelStatus {
  state: "loading" | "ready" | "missing" | "failed";
  path: string;
  message?: string;
}
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
const ENGINE_OF: Record<Route, Engine> = { client: "pc", server: "server", cloud: "cloud" };
const SAMPLE: Record<SetupCleanup, string> = {
  rules: "So lunch is Thursday.",
  pc: "Lunch is on Thursday.",
  server: "Lunch is on Thursday.",
  cloud: "Lunch is on Thursday.",
};
/** The catalogue model behind each row (model_catalog ids). */
const CATALOG_ID: Record<"parakeet" | "whisper", string> = {
  parakeet: "parakeet-tdt-0.6b-v2-int8",
  whisper: "whisper-base",
};

const st = {
  step: "welcome" as Step,
  engine: "pc" as Engine,
  cleanup: "rules" as SetupCleanup,
  /** The user picked a cleanup option on this run (else it follows the saved settings and the server). */
  cleanupPicked: false,
  /** D78: which Cloud provider cleans up (speech is always OpenRouter). */
  prov: "openrouter" as CloudProvider,
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
};
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

/** Steps shown in this build: no model step in a build without engines on this PC. */
function steps(): Step[] {
  return has("pc") ? ["welcome", "engine", "model", "cleanup", "try"] : ["welcome", "engine", "cleanup", "try"];
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
    (b.querySelector("[data-ob-rec]") as HTMLElement).hidden = id !== recommendedEngine(has);
  }
  placeServerForm();
  $<HTMLElement>("ob_cloud_detail").hidden = st.engine !== "cloud";
  $<HTMLElement>("ob_engine_hint").textContent = "";
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
    $<HTMLElement>("ob_host_err").textContent = "Type your server's address.";
    return false;
  }
  try {
    await devStore.save({ ...patch, host, token: $<HTMLInputElement>("ob_token").value.trim() });
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

/** "Connect" on steps 3/4: save the server (downloads only; speech keeps its route), then test it. */
async function connectServer(): Promise<void> {
  if (await saveServer({})) await testServer();
}

/** Step 2's server fields live on step 2, or on step 3/4 while "Your server" is picked with none saved. */
function placeServerForm(): void {
  const form = $<HTMLElement>("ob_server_detail");
  const here = st.step === "model" || st.step === "cleanup" ? st.step : "engine";
  const slot = document.querySelector<HTMLElement>(`#onboarding [data-ob-server-slot="${here}"]`);
  if (slot && form.parentElement !== slot) slot.append(form);
  form.hidden = here === "engine" ? st.engine !== "server" : !srcView().serverForm;
  $<HTMLButtonElement>("ob_test_conn").textContent = here === "engine" ? "Test" : "Connect";
  $<HTMLElement>("ob_server_help").textContent =
    here === "engine" ? "No server yet? Pick This PC above, or" : "No server yet? Download from Hugging Face, or";
}

/** Continue on the engine step: turn the picked engine on and put it first. */
async function applyEngine(): Promise<boolean> {
  const dev = views.dev;
  if (!dev) return true;
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

function activeModel(): "parakeet" | "whisper" {
  return prefsStore.get()?.asr_engine === "parakeet" ? "parakeet" : "whisper";
}

function renderModel(): void {
  const needed = st.engine === "pc";
  const onServer = st.engine === "server";
  $<HTMLElement>("ob_model_optional").hidden = needed;
  const dl = models.canDownload();
  $<HTMLElement>("ob_model_lead").textContent = onServer
    ? "The model is the part that listens. This PC uses it whenever your server can't be reached. We download it once, from your server or from Hugging Face, or you can add the file yourself."
    : !needed
      ? "Your choice does the listening. A model on this PC keeps dictation working when it can't be reached. You can skip this."
      : dl
        ? "The model is the part that listens. We can download it for you once, from your server or from Hugging Face, or you can add the file yourself."
        : "The model is the part that listens. Get it once from where its makers publish it, then tell Murmur where it is.";
  $<HTMLElement>("ob_model_note").textContent = dl
    ? "A download only fetches the file. Nothing about you or your voice is sent. Every file is checked against its SHA-256 fingerprint before it's used, wherever it came from."
    : "Murmur never downloads anything by itself. Your voice is turned into text by the model on this PC.";
  const v = renderSource("model", dl);
  $<HTMLElement>("ob_model_links").hidden = !v.hfLinks;
  const p = prefsStore.get();
  if (p) {
    const pk = $<HTMLInputElement>("ob_parakeet_dir");
    const wh = $<HTMLInputElement>("ob_whisper_path");
    if (document.activeElement !== pk) pk.value = p.parakeet_model_dir ?? "";
    if (document.activeElement !== wh) wh.value = p.model_path ?? "";
  }
  if (st.info) {
    $<HTMLElement>("ob_models_dir").textContent = st.info.models_dir;
    $<HTMLInputElement>("ob_parakeet_dir").placeholder = st.info.default_parakeet_dir;
    $<HTMLInputElement>("ob_whisper_path").placeholder = st.info.default_path;
  }
  const active = activeModel();
  const m = st.model;
  for (const r of document.querySelectorAll<HTMLElement>("#onboarding [data-ob-model]")) {
    const id = r.dataset.obModel;
    const on = id === active;
    r.classList.toggle("is-on", on);
    (r.querySelector("[data-ob-rec]") as HTMLElement).hidden = id !== "parakeet";
    const badge = r.querySelector<HTMLElement>("[data-ob-model-badge]") as HTMLElement;
    const line = r.querySelector<HTMLElement>("[data-ob-model-status]") as HTMLElement;
    const use = r.querySelector<HTMLButtonElement>("[data-ob-use]") as HTMLButtonElement;
    use.textContent = on ? "Check again" : "Use";
    renderDownload(r, id as "parakeet" | "whisper", v);
    badge.hidden = !on || !m;
    line.textContent = "";
    line.className = "lw-ob-model-status";
    if (!on || !m) continue;
    const [text, kind, desc] =
      m.state === "ready"
        ? ["Ready", "success", `Found and loaded: ${m.path}`]
        : m.state === "loading"
          ? ["Loading", "light", `Loading ${m.path}…`]
          : m.state === "missing"
            ? ["Not found", "warning", `Nothing at ${m.path}.`]
            : ["Failed", "destructive", `Could not load ${m.path}: ${m.message ?? ""}`];
    badge.textContent = text;
    badge.className = `fl-badge fl-badge-${kind}`;
    line.textContent = desc;
    line.classList.add(m.state === "ready" ? "ok" : m.state === "loading" ? "hint" : "err");
  }
  const downloading = models.busy();
  const foot = modelStepFoot({ needed, serverEngine: onServer, ready: !!m && (m.state === "ready" || m.state === "loading"), busy: downloading });
  const next = nextBtn("model");
  next.hidden = foot.next === "hidden";
  next.disabled = foot.next === "disabled";
  $<HTMLElement>("ob_model_skip").hidden = !foot.skip;
  $<HTMLElement>("ob_model_hint").textContent = foot.next === "disabled"
    ? "Add a speech model to continue"
    : foot.next === "hidden"
      ? ""
      : downloading
      ? "You can carry on. It finishes in the background."
      : m?.state === "loading"
        ? "You can carry on. It finishes loading in the background."
        : "";
}

/** "Download from" over where downloads come from (the backend's catalogue). */
function srcView(): SourceView {
  return sourceView({ pick: st.pick, effective: downloadSource(), remote: !!views.dev?.caps.remote, serverSet: hasServer() });
}

/** Step 3/4's "Download from: Your server | Hugging Face" (D79) and the server fields under it. */
function renderSource(step: "model" | "cleanup", dl: boolean): SourceView {
  const v = srcView();
  const box = panel(step).querySelector<HTMLElement>("[data-ob-src-box]") as HTMLElement;
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

/** "Your server doesn't have this one" description of a catalogue model. */
const missingDesc = (e: CatalogEntry): string =>
  `${e.desc} · ${mb(e.size)}. Your server doesn't have this one. Ask whoever runs it to add it, or switch to Hugging Face.`;

/** A row's Download / Cancel buttons and progress (3b). */
function renderDownload(r: HTMLElement, id: "parakeet" | "whisper", v: SourceView): void {
  const e = models.entry(CATALOG_ID[id]);
  const p = e ? models.progress(e.id) : undefined;
  const busy = !!e && models.busy(e.id);
  const dl = r.querySelector<HTMLButtonElement>("[data-ob-dl]") as HTMLButtonElement;
  const cancel = r.querySelector<HTMLButtonElement>("[data-ob-dl-cancel]") as HTMLButtonElement;
  const use = r.querySelector<HTMLButtonElement>("[data-ob-use]") as HTMLButtonElement;
  const box = r.querySelector<HTMLElement>("[data-ob-dl-box]") as HTMLElement;
  const bar = r.querySelector<HTMLElement>("[data-ob-dl-bar]") as HTMLElement;
  const line = r.querySelector<HTMLElement>("[data-ob-dl-line]") as HTMLElement;
  const desc = r.querySelector<HTMLElement>("[data-ob-model-desc]") as HTMLElement;
  const missing = models.notOnServer(e) && !e?.installed;
  if (e) {
    const what = e.files.length > 1 ? `a folder of ${mb(e.size)}` : `a ${mb(e.size)} file`;
    desc.textContent = missing ? missingDesc(e) : `${e.desc} · ${what}${e.installed ? " · downloaded and checked" : ""}`;
  }
  (r.querySelector("[data-ob-missing]") as HTMLElement).hidden = !missing || busy;
  dl.hidden = !e || !v.downloads || !models.canStart(e) || e.installed || busy;
  dl.disabled = models.busy();
  dl.textContent = p?.state === "error" ? "Try again" : "Download";
  cancel.hidden = !busy;
  use.hidden = busy;
  renderProgress(box, bar, line, e, busy);
}

/** "Choose folder" / "Choose file": the native picker, then Use it. */
async function pickModel(id: "parakeet" | "whisper" | "llm"): Promise<void> {
  try {
    const p =
      id === "parakeet"
        ? await pickPath("folder", "Choose the Parakeet model folder")
        : id === "whisper"
          ? await pickPath("file", "Choose a Whisper model", [{ name: "Whisper model (ggml)", pattern: "*.bin" }, { name: "All files", pattern: "*.*" }])
          : await pickPath("file", "Choose an AI cleanup model", [{ name: "GGUF model", pattern: "*.gguf" }, { name: "All files", pattern: "*.*" }]);
    if (!p) return;
    if (id === "llm") {
      $<HTMLInputElement>("ob_llm_path").value = p;
      return;
    }
    $<HTMLInputElement>(id === "parakeet" ? "ob_parakeet_dir" : "ob_whisper_path").value = p;
    await useModel(id);
  } catch (e) {
    console.error("pick_path", e);
  }
}

/** "Use": save the engine + its location; the app reloads the model and reports model:status. */
async function useModel(id: "parakeet" | "whisper"): Promise<void> {
  const patch: Partial<Prefs> =
    id === "parakeet"
      ? { asr_engine: "parakeet", parakeet_model_dir: $<HTMLInputElement>("ob_parakeet_dir").value.trim() || null }
      : { asr_engine: "whisper", model_path: $<HTMLInputElement>("ob_whisper_path").value.trim() || null };
  try {
    await prefsStore.save(patch);
    st.model = (await invoke<ModelInfo>("get_model_status")).status;
  } catch (e) {
    st.model = { state: "failed", path: "", message: String(e) };
  }
  renderModel();
}

// ------------------------------------------------------------ step: cleanup

function pickCleanup(c: SetupCleanup): void {
  if (!hasCleanup(c)) return;
  st.cleanup = c;
  st.cleanupPicked = true;
  renderCleanup();
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
  renderCloudProvider();
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
  const v = renderSource("cleanup", !!e?.available);
  $<HTMLElement>("ob_llm_hf").hidden = !v.hfLinks;
  const row = $<HTMLElement>("ob_llm_dl");
  row.hidden = !e;
  if (!e) return;
  const busy = models.busy(e.id);
  const missing = models.notOnServer(e) && !e.installed;
  $<HTMLElement>("ob_llm_dl_title").textContent = e.title;
  $<HTMLElement>("ob_llm_dl_desc").textContent = missing
    ? missingDesc(e)
    : `${e.desc} · a ${mb(e.size)} file${e.installed ? " · downloaded and checked" : ""}`;
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
  if (st.step === "model") renderModel();
  else if (st.step === "engine") placeServerForm();
  else if (st.step === "cleanup") {
    renderCleanupOptions();
    renderLlmModel();
  }
}

/** D78: the OpenRouter | LiteLLM switch of the Cloud cleanup option. */
function renderCloudProvider(): void {
  const custom = st.prov === "custom";
  for (const b of document.querySelectorAll<HTMLButtonElement>("#ob_cloud_prov_seg [data-ob-cloud-provider]")) {
    const on = b.dataset.obCloudProvider === st.prov;
    b.setAttribute("aria-checked", String(on));
    b.classList.toggle("is-on", on);
  }
  $<HTMLElement>("ob_cloud_prov_hint").textContent = custom ? "Your own OpenAI-compatible proxy" : "Pay per use with your key";
  $<HTMLElement>("ob_or_key_row").hidden = custom;
  $<HTMLElement>("ob_ll_rows").hidden = !custom;
  $<HTMLElement>("ob_clean_consent_title").textContent = custom
    ? "Send my dictated text to my proxy"
    : "Send my dictated text to OpenRouter";
  $<HTMLElement>("ob_clean_consent_desc").textContent = custom
    ? `Your words go to ${llHost() || "your proxy"} as text. You confirm once for this address; a new address asks again.`
    : "Your words go to openrouter.ai as text, to zero-retention providers only. Your voice is not sent unless Cloud speech to text is also on.";
}

/** The host typed in the LiteLLM address field ("" when none). */
function llHost(): string {
  try {
    return new URL($<HTMLInputElement>("ob_ll_url").value.trim()).host;
  } catch {
    return "";
  }
}

function pickCloudProvider(p: CloudProvider): void {
  if (p === st.prov) return;
  st.prov = p;
  // Consent names a host: switching provider asks again.
  $<HTMLInputElement>("ob_clean_consent").checked = consentFor(p);
  renderCloudProvider();
  status("ob_cleanup_status", "");
}

function consentFor(p: CloudProvider): boolean {
  const c = views.or?.cleanup;
  if (!c) return false;
  if (p === "custom") return c.provider === "custom" && c.custom.consent;
  return c.provider === "openrouter" && c.consent;
}

interface CustomModels {
  models?: string[];
  error?: string;
  message?: string;
}

/** "Test connection": the proxy's model list (typed key, else the saved one). */
async function testProxy(): Promise<void> {
  const out = $<HTMLElement>("ob_ll_test_out");
  const baseUrl = $<HTMLInputElement>("ob_ll_url").value.trim();
  if (!baseUrl) {
    out.textContent = "Enter your proxy's address first.";
    return;
  }
  out.textContent = `Asking ${llHost() || "your proxy"} for its models…`;
  const typed = $<HTMLInputElement>("ob_ll_key").value.trim();
  const args = { baseUrl, key: typed === "" ? null : typed };
  try {
    let t0 = performance.now();
    let r = await invoke<CustomModels>("dev_or_custom_models", { ...args, consent: false });
    // A saved key goes only to a host the user agreed to (D52): the tick is that agreement.
    if (r.error === "consent_needed") {
      if (!$<HTMLInputElement>("ob_clean_consent").checked) {
        out.textContent = "Not sent: tick “Send my dictated text to my proxy” below first, or paste the key here.";
        return;
      }
      t0 = performance.now();
      r = await invoke<CustomModels>("dev_or_custom_models", { ...args, consent: true });
      views.setOr(await invoke<OrView>("dev_or_get"));
    }
    const ms = Math.round(performance.now() - t0);
    if (r.models) {
      $<HTMLDataListElement>("ob_ll_models").replaceChildren(
        ...r.models.map((id) => Object.assign(document.createElement("option"), { value: id })),
      );
      const model = $<HTMLInputElement>("ob_ll_model");
      if (!model.value && r.models.length === 1) model.value = r.models[0] ?? "";
      const n = r.models.length;
      out.textContent = `Connected · ${n} model${n === 1 ? "" : "s"} · ${ms} ms${n ? ": pick one in the Model field" : ""}`;
    } else {
      out.textContent = r.message ?? r.error ?? "No answer";
    }
  } catch (e) {
    out.textContent = errText(e);
  }
}

/** Save the LiteLLM address, model and (typed) key as the Cloud provider. */
async function applyProxy(): Promise<boolean> {
  const baseUrl = $<HTMLInputElement>("ob_ll_url").value.trim();
  const model = $<HTMLInputElement>("ob_ll_model").value.trim();
  if (!baseUrl) {
    status("ob_cleanup_status", "Enter your proxy's address.", false);
    return false;
  }
  if (!model) {
    status("ob_cleanup_status", "Pick a model: Test connection lists them.", false);
    return false;
  }
  // D78: one Cloud provider at a time on Windows, and LiteLLM has no speech.
  if (st.engine === "cloud") {
    status(
      "ob_cleanup_status",
      "A LiteLLM proxy can't do speech to text, and Cloud is your speech engine: pick OpenRouter here, or go back and pick another engine.",
      false,
    );
    return false;
  }
  const typed = $<HTMLInputElement>("ob_ll_key").value.trim();
  if (typed) {
    await invoke<KeyState>("dev_cloud_set_custom_key", { key: typed });
    $<HTMLInputElement>("ob_ll_key").value = "";
  }
  const c = views.or?.cleanup.custom;
  views.setOr(
    await invoke<OrView>("dev_or_set_custom_cleanup", {
      form: { provider: "custom", base_url: baseUrl, model, timeout_ms: c?.timeout_ms ?? 4000, consent: true },
    }),
  );
  return true;
}

/** Back to OpenRouter when the saved provider is LiteLLM (its fields are kept). */
async function useOpenRouter(): Promise<void> {
  const c = views.or?.cleanup;
  if (!c || c.provider === "openrouter") return;
  views.setOr(
    await invoke<OrView>("dev_or_set_custom_cleanup", {
      form: {
        provider: "openrouter",
        base_url: c.custom.base_url,
        model: c.custom.model,
        timeout_ms: c.custom.timeout_ms,
        consent: false,
      },
    }),
  );
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
  const level = p.cleanup_level === "none" ? "light" : p.cleanup_level;
  try {
    if (st.cleanup === "cloud") {
      if (!$<HTMLInputElement>("ob_clean_consent").checked) {
        status("ob_cleanup_status", `Tick “${$<HTMLElement>("ob_clean_consent_title").textContent ?? ""}” to use it.`, false);
        return false;
      }
      if (st.prov === "custom") {
        if (!(await applyProxy())) return false;
      } else {
        if (!(await saveKey("ob_clean_key", "ob_cleanup_status"))) return false;
        await useOpenRouter();
      }
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
        llm_model_path: $<HTMLInputElement>("ob_llm_path").value.trim() || null,
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
  cloud: "in the cloud",
};

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
  const where = WHERE[last.engine ?? ""] ?? (last.engine ? `by ${last.engine}` : "");
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

function show(s: Step): void {
  st.step = s;
  for (const p of document.querySelectorAll<HTMLElement>("#onboarding [data-ob-step]")) p.hidden = p.dataset.obStep !== s;
  const list = steps();
  const n = list.indexOf(s) + 1;
  const over = panel(s).querySelector<HTMLElement>(".fl-overline");
  if (over) over.textContent = `Step ${n} of ${list.length}`;
  if (s === "engine") renderEngine();
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

/** "Skip — use my server only": on to the next step with no model on this PC. */
function skipModel(): void {
  if (st.step !== "model") return;
  const list = steps();
  show(list[list.indexOf("model") + 1]);
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

// ------------------------------------------------------------ choices from the saved settings

function initialChoices(): void {
  const dev = views.dev?.form;
  const or = views.or;
  const first = dev?.route_order.find((r) =>
    r === "client" ? dev.allow_client : r === "server" ? dev.allow_server && !!dev.host : !!or?.stt.allow,
  );
  const p = prefsStore.get();
  st.engine = initialEngine(first ? ENGINE_OF[first] : null, !!p?.onboarded, has);
  st.cleanup = savedCleanup();
  st.cleanupPicked = false;
  $<HTMLInputElement>("ob_host").value = dev?.host ?? "";
  $<HTMLInputElement>("ob_token").placeholder = dev?.token_saved ? "saved (hidden), leave empty to keep" : "from murmur-server gen-token";
  $<HTMLInputElement>("ob_stt_consent").checked = !!or?.stt.consent;
  st.prov = or?.cleanup.provider ?? "openrouter";
  $<HTMLInputElement>("ob_clean_consent").checked = consentFor(st.prov);
  $<HTMLInputElement>("ob_ll_url").value = or?.cleanup.custom.base_url ?? "";
  $<HTMLInputElement>("ob_ll_model").value = or?.cleanup.custom.model ?? "";
  if (or?.cleanup.custom.key.set) {
    $<HTMLInputElement>("ob_ll_key").placeholder = `saved ${or.cleanup.custom.key.masked ?? ""}`;
    $<HTMLElement>("ob_ll_key_desc").textContent = "A key is saved. Paste a new one only to replace it.";
  }
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
      else if (act === "skip-model") skipModel();
      const eng = t.closest<HTMLElement>("[data-ob-engine]")?.dataset.obEngine;
      if (eng) pickEngine(eng as Engine);
      const cl = t.closest<HTMLElement>("[data-ob-cleanup]")?.dataset.obCleanup;
      if (cl) pickCleanup(cl as SetupCleanup);
      const use = t.closest<HTMLElement>("[data-ob-use]")?.dataset.obUse;
      if (use) void useModel(use as "parakeet" | "whisper");
      const pick = t.closest<HTMLElement>("[data-ob-pick]")?.dataset.obPick;
      if (pick) void pickModel(pick as "parakeet" | "whisper" | "llm");
      const src = t.closest<HTMLElement>("[data-ob-src]")?.dataset.obSrc;
      if (src) pickSource(src as ModelSource);
      const dl = t.closest<HTMLElement>("[data-ob-dl]")?.dataset.obDl;
      if (dl) void models.confirmAndStart(CATALOG_ID[dl as "parakeet" | "whisper"]);
      const stop = t.closest<HTMLElement>("[data-ob-dl-cancel]")?.dataset.obDlCancel;
      if (stop) void models.cancel(CATALOG_ID[stop as "parakeet" | "whisper"]);
    });
    // Enter in a model path field = Use.
    for (const [id, m] of [["ob_parakeet_dir", "parakeet"], ["ob_whisper_path", "whisper"]] as const) {
      $<HTMLInputElement>(id).addEventListener("keydown", (e) => {
        if (e.key === "Enter") void useModel(m);
      });
    }
    $<HTMLInputElement>("ob_stt_consent").addEventListener("change", () => status("ob_cloud_status", ""));
    $<HTMLInputElement>("ob_clean_consent").addEventListener("change", () => status("ob_cleanup_status", ""));
    for (const b of document.querySelectorAll<HTMLButtonElement>("#ob_cloud_prov_seg [data-ob-cloud-provider]")) {
      b.addEventListener("click", () => pickCloudProvider((b.dataset.obCloudProvider ?? "openrouter") as CloudProvider));
    }
    $<HTMLButtonElement>("ob_ll_test").addEventListener("click", () => void testProxy());
    $<HTMLInputElement>("ob_ll_url").addEventListener("input", renderCloudProvider);
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
    st.pick = null;
    $<HTMLElement>("onboarding").hidden = false;
    document.body.classList.add("lw-ob-open");
    show("welcome");
  },
};
