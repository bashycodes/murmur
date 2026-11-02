// "Record this meeting?" (meeting detection, D48). src/app.rs opens this
// window as `meeting-prompt.html?app=Zoom` when a call starts and the
// setting is "Ask me", and closes it when the call ends. The only command
// it may call is `meeting_prompt_answer` (capabilities/meeting-prompt.json).

import { invoke } from "./lib/ipc.js";
import { followSystemTheme } from "./lib/theme.js";

/** Not answered within this long = "Not now" (the call goes unrecorded). */
const TIMEOUT_MS = 60_000;

followSystemTheme();

const app = new URLSearchParams(location.search).get("app") || "A meeting app";
const body = document.getElementById("mp-body") as HTMLParagraphElement;
body.textContent = `${app} call started. Your mic is “Me”, the call's sound is “Others”.`;

let answered = false;
function answer(record: boolean): void {
  if (answered) return;
  answered = true;
  void invoke("meeting_prompt_answer", { record });
}

(document.getElementById("mp-yes") as HTMLButtonElement).addEventListener("click", () => answer(true));
(document.getElementById("mp-no") as HTMLButtonElement).addEventListener("click", () => answer(false));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") answer(false);
});
setTimeout(() => answer(false), TIMEOUT_MS);
