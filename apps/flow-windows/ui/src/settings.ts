// Settings window shell: tab nav, the shared Save form (General, AI cleanup
// and Style own their prefs fields; the prefs store saves the patch) and
// boot. Each page module renders its own DOM into its container in
// settings.html; see ui/src/pages/*.ts and ui/src/lib/*.ts.

import { $, setStatus } from "./lib/dom.js";
import { onEvent } from "./lib/ipc.js";
import { prefsStore, type Prefs } from "./lib/prefs-store.js";
import { router, type Page } from "./lib/router.js";
import { views } from "./lib/views.js";
import { aiPage, collectAi } from "./pages/ai.js";
import { developerPage } from "./pages/developer.js";
import { dictionaryPage } from "./pages/dictionary.js";
import { collectGeneral, generalPage } from "./pages/general.js";
import { historyPage } from "./pages/history.js";
import { insightsPage } from "./pages/insights.js";
import { openrouterPage } from "./pages/openrouter.js";
import { collectStyle, stylePage } from "./pages/style.js";

const PAGES: Page[] = [
  generalPage,
  aiPage,
  stylePage,
  historyPage,
  insightsPage,
  dictionaryPage,
  openrouterPage,
  developerPage,
];

function showPrivacyLine(): void {
  const line = $<HTMLElement>("privacy_line");
  if (views.or?.supported) {
    line.textContent =
      "Speech is processed on this PC or on your own server, and by OpenRouter only if you turn it on (OpenRouter tab).";
  } else if (views.dev?.caps.remote) {
    line.textContent = "Speech is processed on this PC or on your own server. Nothing goes to a third party.";
  } else {
    line.textContent = "Everything runs on this PC. Nothing is sent over the network.";
  }
}

async function saveForm(ev: Event): Promise<void> {
  ev.preventDefault();
  if (!prefsStore.get()) return;
  // AI last: Verbatim switches Auto-format (a General field) off.
  const patch: Partial<Prefs> = { ...collectGeneral(), ...collectStyle(), ...collectAi() };
  try {
    await prefsStore.save(patch);
    setStatus("status", "Saved", true);
  } catch (e) {
    setStatus("status", String(e), false);
  }
}

for (const p of PAGES) {
  router.register(p);
  p.mount($<HTMLElement>(`page_${p.id}`));
}

router.onNavigate((name) => {
  for (const b of document.querySelectorAll<HTMLButtonElement>("nav.tabs button")) {
    b.setAttribute("aria-selected", String(b.dataset.tab === name));
  }
  for (const p of document.querySelectorAll<HTMLElement>("[data-panel]")) {
    p.hidden = p.dataset.panel !== name;
  }
  $<HTMLFormElement>("form").hidden = name !== "general" && name !== "ai";
});

views.onDev((v) => {
  $<HTMLElement>("tab_developer").hidden = !v.visible;
  showPrivacyLine();
  // Server-only builds have no local model: no model paths or LLM files.
  for (const f of document.querySelectorAll<HTMLElement>("[data-needs=local]")) f.hidden = !v.caps.local;
});
views.onOr((v) => {
  $<HTMLElement>("tab_openrouter").hidden = !v || (!v.supported && !v.notice);
  $<HTMLElement>("or_badge").hidden = !v?.notice;
  showPrivacyLine();
});

for (const b of document.querySelectorAll<HTMLButtonElement>("nav.tabs button")) {
  b.addEventListener("click", () => router.go(b.dataset.tab ?? "general"));
}
$<HTMLFormElement>("form").addEventListener("submit", (e) => void saveForm(e));
void onEvent<string>("settings:tab", (e) => router.go(e.payload));

async function load(): Promise<void> {
  await prefsStore.load();
  for (const p of PAGES) {
    try {
      await p.load?.();
    } catch (e) {
      console.error(`page ${p.id}: load failed`, e);
    }
  }
}

router.go("general");
void load();
