"use strict";
// Flow bar pill, styled after the design artboard
// docs/design/app-design/FlowBar.dc.html; behaviour matched to Wispr
// 1.6.1167 (windows-research findings/data/overlay-spec.json). The window
// is a fixed 512x614 transparent canvas; states are pure CSS on
// #pill[data-state] (+ data-done while the engine badge shows).
// Backend events: `dictation:status` ({state, message?}), `audio:level`
// (0..1, ~every 150 ms), `dictation:preview`, `dictation:route`,
// `dictation:live-route`, `dictation:max-warning` (ms left).
// Bar heights (px) from the artboard's waveform; the mic level scales them.
const PROFILE = [6, 10, 16, 12, 20, 14, 8, 18, 22, 12, 9, 16, 11, 6, 14, 10, 7, 5];
const GAIN = 5; // --level = min(1, 5 * v) for dictation
const pill = document.getElementById("pill");
const wave = document.getElementById("wave");
const msg = document.getElementById("msg");
const preview = document.getElementById("preview");
PROFILE.forEach((h, i) => {
    const b = document.createElement("span");
    b.className = "bar";
    b.style.height = `${h}px`;
    b.style.animationDelay = `${(-0.13 * ((i * 7) % 9)).toFixed(2)}s`;
    wave.appendChild(b);
});
let pillState = "resting";
// Cloud failure notice (see `showCloudNotice`); declared before the first apply().
let noticeText = "";
let noticeUntil = 0;
let target = 0;
let v = 0;
function isActive() {
    return pillState === "active_ptt" || pillState === "active_locked";
}
function frame() {
    // EMA per frame, floored to 2 decimals.
    v = Math.floor((0.85 * v + 0.15 * target) * 100) / 100;
    pill.style.setProperty("--level", String(Math.min(1, GAIN * v)));
    requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
function apply(s) {
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
    }
    else {
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
    }
    else {
        delete preview.dataset.kind;
        showCloudNotice();
    }
    if (isActive() && !wasActive) {
        pill.dataset.mic = "inactive"; // dim, still bars until real audio arrives
    }
    if (!isActive())
        showMaxWarning(0);
    syncDone();
}
function showPreview(text) {
    preview.textContent = text;
    preview.classList.toggle("visible", text.length > 0 && isActive());
}
const { listen } = window.__TAURI__.event;
const overlayInvoke = window.__TAURI__.core.invoke;
// ✕ / ✓ in hands-free mode. The window never takes focus (WS_EX_NOACTIVATE),
// so clicking does not steal the caret from the target app.
for (const [id, action] of [
    ["btn-cancel", "cancel"],
    ["btn-finish", "finish"],
]) {
    document.getElementById(id)?.addEventListener("mousedown", (e) => {
        e.preventDefault();
        if (pillState === "active_locked")
            void overlayInvoke("overlay_action", { action });
    });
}
void listen("dictation:preview", (e) => showPreview(e.payload));
void listen("dictation:status", (e) => apply(e.payload));
void listen("audio:level", (e) => {
    if (!isActive())
        return;
    target = Math.max(0, Math.min(1, e.payload));
    if (target > 0)
        pill.dataset.mic = "active";
});
// Which engine served the last dictation: S = your server, L = this PC
// (Developer setting "show engine badge"), C = a cloud provider (always,
// D18 C7); red dot = it fell back. A cloud failure ("Cloud STT failed
// (key rejected); used this PC") shows in the bubble above the pill.
const badge = document.getElementById("engine-badge");
let badgeTimer = 0;
// "Done, with engine": the pill shows ✓ + letter while the badge is up and
// nothing newer (recording / working) is on the pill.
function syncDone() {
    const done = !badge.hidden && (pillState === "resting" || pillState === "notice");
    if (done)
        pill.dataset.done = "on";
    else
        delete pill.dataset.done;
}
void listen("dictation:route", (e) => {
    clearTimeout(badgeTimer);
    if (!e.payload) {
        badge.hidden = true;
        syncDone();
        return;
    }
    badge.textContent = e.payload.letter;
    badge.dataset.letter = e.payload.letter;
    badge.classList.toggle("fallback", e.payload.fallback);
    badge.title = e.payload.title ?? (e.payload.fallback ? "fell back" : "");
    badge.hidden = false;
    syncDone();
    badgeTimer = setTimeout(() => {
        badge.hidden = true;
        syncDone();
    }, 4000);
    if (e.payload.message) {
        noticeText = e.payload.message;
        noticeUntil = Date.now() + 4000;
        showCloudNotice();
        setTimeout(showCloudNotice, 4050);
    }
});
// The route event arrives while the pill is still "processing"; the
// notice survives the switch to resting/pasted and expires after 4 s.
function showCloudNotice() {
    if (!noticeText || isActive())
        return;
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
const routeLabel = document.getElementById("route-label");
void listen("dictation:live-route", (e) => {
    const r = e.payload;
    routeLabel.hidden = !r;
    document.body.classList.toggle("has-route", !!r);
    routeLabel.textContent = r ? r.text : "";
    routeLabel.title = r?.title ?? "";
    if (r)
        routeLabel.dataset.engine = r.engine;
    else
        delete routeLabel.dataset.engine;
});
// Max dictation length: the controller warns with the time left; count it
// down on a chip above the pill until the recording ends.
const maxWarn = document.getElementById("max-warn");
let maxWarnTimer = 0;
let maxWarnEnd = 0;
function showMaxWarning(remainingMs) {
    clearInterval(maxWarnTimer);
    maxWarnEnd = Date.now() + remainingMs;
    const tick = () => {
        const left = Math.ceil((maxWarnEnd - Date.now()) / 1000);
        if (left <= 0 || !isActive()) {
            clearInterval(maxWarnTimer);
            maxWarn.hidden = true;
            return;
        }
        maxWarn.textContent = `Recording stops in ${left} s`;
        maxWarn.hidden = false;
    };
    tick();
    if (!maxWarn.hidden)
        maxWarnTimer = setInterval(tick, 250);
}
void listen("dictation:max-warning", (e) => showMaxWarning(e.payload));
// Last: apply() touches the badge / countdown elements declared above.
apply({ state: "resting" });
