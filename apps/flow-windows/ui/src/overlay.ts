// Flow bar pill, matched to Wispr 1.6.1167 (windows-research
// findings/data/overlay-spec.json). The window is a fixed 512x614
// transparent canvas; states are pure CSS on #pill[data-state].
// Backend events: `dictation:status` ({state, message?}) and
// `audio:level` (0..1, ~every 150 ms).

type OverlayState =
  | { state: "resting" }
  | { state: "active_ptt" }
  | { state: "active_locked" }
  | { state: "active_command" }
  | { state: "processing" }
  | { state: "command_processing" }
  | { state: "error"; message: string }
  | { state: "notice"; message: string };

const BAR_COUNT = 10;
const GAIN = 5; // --audio-scale = max(1, 5 * v) for dictation

const pill = document.getElementById("pill") as HTMLDivElement;
const wave = document.getElementById("wave") as HTMLDivElement;
const msg = document.getElementById("msg") as HTMLDivElement;
const spinner = document.getElementById("spinner") as HTMLDivElement;
const preview = document.getElementById("preview") as HTMLDivElement;

// Bar heights g = max(0, 1 - h^2/48), h = |(n-1)/2 - i|; phase delays 0.1*m[i] s.
const PHASE = [0, 1, 2, 3, 4, -5, -4, -3, -2, -1];
for (let i = 0; i < BAR_COUNT; i++) {
  const h = Math.abs((BAR_COUNT - 1) / 2 - i);
  const b = document.createElement("span");
  b.className = "bar";
  b.style.setProperty("--bar-height-scale", Math.max(0, 1 - (h * h) / 48).toFixed(3));
  b.style.animationDelay = `${(0.1 * PHASE[i]).toFixed(1)}s`;
  wave.appendChild(b);
}
for (let i = 0; i < 8; i++) {
  const s = document.createElement("i");
  s.style.transform = `rotate(${45 * i}deg) translate(0, -6px)`;
  s.style.animationDelay = `${(-0.1375 * (7 - i)).toFixed(4)}s`;
  spinner.appendChild(s);
}

// Command Mode reuses the push-to-talk / processing pill with an orange
// accent (#pill[data-command]), so every other rule keeps working.
type PillState = Exclude<OverlayState["state"], "active_command" | "command_processing">;
let pillState: PillState = "resting";
// Cloud failure notice (see `showCloudNotice`); declared before the first apply().
let noticeText = "";
let noticeUntil = 0;
let target = 0;
let v = 0;

function isActive(): boolean {
  return pillState === "active_ptt" || pillState === "active_locked";
}

function frame(): void {
  // EMA per frame, floored to 2 decimals.
  v = Math.floor((0.85 * v + 0.15 * target) * 100) / 100;
  pill.style.setProperty("--audio-scale", String(Math.max(1, GAIN * v)));
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

function apply(s: OverlayState): void {
  const wasActive = isActive();
  pillState =
    s.state === "active_command"
      ? "active_ptt"
      : s.state === "command_processing"
        ? "processing"
        : s.state;
  pill.dataset.state = pillState;
  if (s.state === "active_command" || s.state === "command_processing") {
    pill.dataset.command = "on";
  } else {
    delete pill.dataset.command;
  }
  const message = s.state === "error" || s.state === "notice" ? s.message : "";
  msg.textContent = message;
  if (!isActive()) {
    target = 0;
    v = 0;
    showPreview("");
  }
  // Error / notice text ("Dictation disabled — …", "No speech heard")
  // shows in the bubble above the pill for as long as the pill lingers.
  if (message) {
    preview.textContent = message;
    preview.dataset.kind = s.state;
    preview.classList.add("visible");
  } else {
    delete preview.dataset.kind;
    showCloudNotice();
  }
  if (isActive() && !wasActive) {
    pill.dataset.mic = "inactive"; // dim, still bars until real audio arrives
  }
}

function showPreview(text: string): void {
  preview.textContent = text;
  preview.classList.toggle("visible", text.length > 0 && isActive());
}

apply({ state: "resting" });

const { listen } = window.__TAURI__.event;
const overlayInvoke = window.__TAURI__.core.invoke;

// ✕ / ✓ in hands-free mode. The window never takes focus (WS_EX_NOACTIVATE),
// so clicking does not steal the caret from the target app.
for (const [id, action] of [
  ["btn-cancel", "cancel"],
  ["btn-finish", "finish"],
] as const) {
  document.getElementById(id)?.addEventListener("mousedown", (e) => {
    e.preventDefault();
    if (pillState === "active_locked") void overlayInvoke("overlay_action", { action });
  });
}

void listen<string>("dictation:preview", (e) => showPreview(e.payload));
void listen<OverlayState>("dictation:status", (e) => apply(e.payload));
void listen<number>("audio:level", (e) => {
  if (!isActive()) return;
  target = Math.max(0, Math.min(1, e.payload));
  if (target > 0) pill.dataset.mic = "active";
});

// Which engine served the last dictation: S = your server, L = this PC
// (Developer setting "show engine badge"), C = a cloud provider (always,
// D18 C7); red dot = it fell back. A cloud failure ("Cloud STT failed
// (key rejected); used this PC") shows in the bubble above the pill.
const badge = document.getElementById("engine-badge") as HTMLSpanElement;
let badgeTimer = 0;
interface Badge {
  letter: string;
  fallback: boolean;
  title?: string | null;
  message?: string | null;
}
void listen<Badge | null>("dictation:route", (e) => {
  clearTimeout(badgeTimer);
  if (!e.payload) {
    badge.hidden = true;
    return;
  }
  badge.textContent = e.payload.letter;
  badge.classList.toggle("fallback", e.payload.fallback);
  badge.title = e.payload.title ?? (e.payload.fallback ? "fell back" : "");
  badge.hidden = false;
  badgeTimer = setTimeout(() => (badge.hidden = true), 4000);
  if (e.payload.message) {
    noticeText = e.payload.message;
    noticeUntil = Date.now() + 4000;
    showCloudNotice();
    setTimeout(showCloudNotice, 4050);
  }
});

// The route event arrives while the pill is still "processing"; the
// notice survives the switch to resting/pasted and expires after 4 s.
function showCloudNotice(): void {
  if (!noticeText || isActive()) return;
  if (Date.now() >= noticeUntil) {
    if (preview.textContent === noticeText) {
      preview.classList.remove("visible");
      delete preview.dataset.kind;
    }
    noticeText = "";
    return;
  }
  preview.textContent = noticeText;
  preview.dataset.kind = "notice";
  preview.classList.add("visible");
}

// Live route while recording/processing (router `poll().route`):
// "Transcribing via this PC… / server… / Groq (model)…", relabelled when a
// failover switches route mid-dictation. `null` = hide.
const routeLabel = document.getElementById("route-label") as HTMLDivElement;
interface LiveRoute {
  text: string;
  engine: string;
  title?: string | null;
}
void listen<LiveRoute | null>("dictation:live-route", (e) => {
  const r = e.payload;
  routeLabel.hidden = !r;
  document.body.classList.toggle("has-route", !!r);
  routeLabel.textContent = r ? r.text : "";
  routeLabel.title = r?.title ?? "";
  if (r) routeLabel.dataset.engine = r.engine;
  else delete routeLabel.dataset.engine;
});
