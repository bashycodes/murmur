// Flow bar pill: 5 visual states + RMS bars (05 §7.3-7.4).
// Backend emits `dictation:status` ({state, message?}) and `audio:level`
// (0..1 scalar at ~30 Hz). Bars are DOM elements scaled via a CSS variable;
// the level is EMA-smoothed per animation frame (α≈0.85, frame-rate
// compensated) like Wispr's status renderer.

type OverlayState =
  | { state: "resting" }
  | { state: "active_ptt" }
  | { state: "active_locked" }
  | { state: "processing" }
  | { state: "error"; message: string };

const BAR_SHAPE = [0.35, 0.55, 0.8, 1.0, 0.9, 1.0, 0.8, 0.55, 0.35];

const pill = document.getElementById("pill") as HTMLDivElement;
const bars = document.getElementById("bars") as HTMLDivElement;
const msg = document.getElementById("msg") as HTMLDivElement;

for (const h of BAR_SHAPE) {
  const b = document.createElement("span");
  b.className = "bar";
  b.style.setProperty("--bar-height-scale", String(h));
  bars.appendChild(b);
}

let target = 0;
let shown = 0;
let last = performance.now();

function frame(now: number): void {
  const dt = Math.min(100, now - last);
  last = now;
  // α=0.85 at 60 fps; compensate for other frame rates.
  const alpha = Math.pow(0.85, dt / 16.67);
  const next = alpha * shown + (1 - alpha) * target;
  // Slew limit (1.6 units/s) keeps the bars from jittering.
  const maxStep = (1.6 * dt) / 1000;
  shown = shown + Math.max(-maxStep, Math.min(maxStep, next - shown));
  pill.style.setProperty("--audio-scale", (0.15 + 0.85 * shown).toFixed(3));
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

function apply(s: OverlayState): void {
  pill.dataset.state = s.state;
  msg.textContent = s.state === "error" ? s.message : "";
  if (s.state !== "active_ptt" && s.state !== "active_locked") {
    target = 0;
  }
}

apply({ state: "resting" });

const { listen } = window.__TAURI__.event;
void listen<OverlayState>("dictation:status", (e) => apply(e.payload));
void listen<number>("audio:level", (e) => {
  target = Math.max(0, Math.min(1, e.payload));
});
