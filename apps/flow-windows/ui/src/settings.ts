// The app window's shell: the sidebar (main pages, the Engines card,
// Settings), the main panel and the Settings dialog with its own
// sub-navigation. Every page renders its own template into an empty
// container made here (ui/src/pages/**); logic shared by several settings
// pages lives in ui/src/features/*. Prefs fields marked data-save="prefs"
// save on change through the prefs store (the only caller of set_prefs).

import { enginesCard } from "./engines-card.js";
import { ai, collectAi } from "./features/ai.js";
import { engines, versionTapped } from "./features/engines.js";
import { collectGeneral, general } from "./features/general.js";
import { openrouter } from "./features/openrouter.js";
import { $, el, html, setStatus } from "./lib/dom.js";
import { ICON, iconEl } from "./lib/icons.js";
import { onEvent } from "./lib/ipc.js";
import { prefsStore, type Prefs } from "./lib/prefs-store.js";
import { router, type Feature, type Page } from "./lib/router.js";
import { followSystemTheme } from "./lib/theme.js";
import { views } from "./lib/views.js";
import { onboarding } from "./onboarding/index.js";
import { dictionaryPage, snippetsPage } from "./pages/dictionary.js";
import { homePage } from "./pages/home.js";
import { insightsPage } from "./pages/insights.js";
import { notetakerPage, scratchpadPage } from "./pages/scratchpad.js";
import { settingsCleanupPage } from "./pages/settings/cleanup.js";
import { settingsDeveloperPage } from "./pages/settings/developer.js";
import { settingsGeneralPage } from "./pages/settings/general.js";
import { settingsOpenrouterPage } from "./pages/settings/openrouter.js";
import { settingsPrivacyPage } from "./pages/settings/privacy.js";
import { settingsServerPage } from "./pages/settings/server.js";
import { settingsSpeechPage } from "./pages/settings/speech.js";
import { settingsSystemPage } from "./pages/settings/system.js";
import { stylePage } from "./pages/style.js";
import { transformsPage } from "./pages/transforms.js";

followSystemTheme();

const PAGES: Page[] = [
  homePage,
  insightsPage,
  dictionaryPage,
  snippetsPage,
  stylePage,
  transformsPage,
  scratchpadPage,
  notetakerPage,
  settingsGeneralPage,
  settingsSystemPage,
  settingsPrivacyPage,
  settingsSpeechPage,
  settingsCleanupPage,
  settingsServerPage,
  settingsOpenrouterPage,
  settingsDeveloperPage,
];

const FEATURES: Feature[] = [general, ai, engines, openrouter];

interface NavItem {
  page?: string;
  label: string;
  icon: string;
  /** Element id (for pages shown only in some builds). */
  id?: string;
  /** Id of a count badge filled by the page. */
  badge?: string;
  /** Not built yet: a "Later" badge (disabled unless it has a page). */
  later?: boolean;
}

const MAIN_NAV: NavItem[] = [
  { page: "home", label: "Dictation", icon: ICON.dictation },
  { page: "notetaker", label: "Notetaker", icon: ICON.notetaker, later: true },
  { page: "insights", label: "Insights", icon: ICON.insights },
  { page: "dictionary", label: "Dictionary", icon: ICON.dictionary, badge: "sugg_badge" },
  { page: "snippets", label: "Snippets", icon: ICON.snippets },
  { page: "style", label: "Style", icon: ICON.style },
  { page: "transforms", label: "Transforms", icon: ICON.cleanup },
  { page: "scratchpad", label: "Scratchpad", icon: "M5 4h14v16H5zM8.5 9h7M8.5 13h4" },
];

const SETTINGS_NAV: [string, NavItem[], string?][] = [
  [
    "Settings",
    [
      { page: "settings-general", label: "General", icon: ICON.general },
      { page: "settings-system", label: "System", icon: ICON.system },
      { page: "settings-privacy", label: "Data and privacy", icon: ICON.privacy },
    ],
  ],
  [
    "Models",
    [
      { page: "settings-speech", label: "Speech to text", icon: ICON.dictation },
      { page: "settings-cleanup", label: "AI cleanup", icon: ICON.cleanup },
    ],
  ],
  [
    "Connections",
    [
      { page: "settings-server", label: "Your server", icon: ICON.server, id: "tab_server" },
      { page: "settings-openrouter", label: "OpenRouter", icon: ICON.cloud, id: "tab_openrouter", badge: "or_badge" },
    ],
    "nav_connections",
  ],
  ["Developer", [{ page: "settings-developer", label: "Developer", icon: ICON.developer, id: "tab_developer" }], "nav_developer"],
];

/** Old tray / event tab names -> pages (the tray still sends `settings:tab`). */
const OLD_TABS: Record<string, string> = {
  general: "settings-general",
  ai: "settings-cleanup",
  history: "home",
  insights: "insights",
  dictionary: "dictionary",
  openrouter: "settings-openrouter",
  developer: "settings-developer",
};

function navButton(n: NavItem): HTMLButtonElement {
  const b = el("button", "fl-nav");
  b.type = "button";
  if (n.id) b.id = n.id;
  if (n.page) b.dataset.page = n.page;
  b.append(iconEl(n.icon, 20), el("span", "fl-nav-label", n.label));
  if (n.badge) {
    const badge = el("span", "fl-badge fl-badge-dark", n.badge === "or_badge" ? "!" : "");
    badge.id = n.badge;
    badge.hidden = true;
    b.append(badge);
  }
  if (n.later) {
    b.disabled = !n.page;
    b.classList.add("is-later");
    b.title = "Not built yet";
    b.append(el("span", "fl-badge fl-badge-light", "Later"));
  }
  return b;
}

function buildNav(): void {
  $<HTMLElement>("main_nav").append(...MAIN_NAV.map(navButton));
  const settings = navButton({ page: "settings", label: "Settings", icon: ICON.settings });
  settings.id = "nav_settings";
  $<HTMLElement>("foot_nav").append(settings);
  const top = $<HTMLElement>("settings_nav");
  SETTINGS_NAV.forEach(([title, items, id], i) => {
    const sec = el("div", "lw-nav-group");
    if (id) sec.id = id;
    if (i > 0) sec.append(el("div", "fl-nav-rule"));
    sec.append(el("div", "fl-nav-section", title), ...items.map(navButton));
    top.append(sec);
  });
}

/** Render every page into its own empty container, then wire. */
function mountPages(): void {
  for (const p of PAGES) {
    router.register(p);
    const root = el("section", "lw-page-root");
    root.id = `page_${p.id}`;
    root.dataset.pageRoot = p.id;
    root.hidden = true;
    root.append(html(p.template));
    $<HTMLElement>(p.area === "main" ? "main" : "settings_body").append(root);
  }
  for (const p of PAGES) p.init?.();
  for (const f of FEATURES) f.init();
  enginesCard.init();
}

// ------------------------------------------------------------ navigation

let lastMain = "home";
let lastSettings = "settings-general";

function showPage(area: "main" | "settings", id: string): void {
  for (const r of document.querySelectorAll<HTMLElement>("[data-page-root]")) {
    const p = PAGES.find((x) => x.id === r.dataset.pageRoot);
    if (p?.area === area) r.hidden = r.dataset.pageRoot !== id;
  }
}

function markNav(): void {
  const settingsOpen = !$<HTMLElement>("settings").hidden;
  for (const b of document.querySelectorAll<HTMLButtonElement>(".lw-sidebar [data-page]")) {
    const on = b.dataset.page === "settings" ? settingsOpen : !settingsOpen && b.dataset.page === lastMain;
    b.classList.toggle("is-active", on);
    if (on) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
  for (const b of document.querySelectorAll<HTMLButtonElement>("#settings_nav [data-page]")) {
    const on = b.dataset.page === lastSettings;
    b.classList.toggle("is-active", on);
    if (on) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
}

router.onNavigate((id) => {
  const p = PAGES.find((x) => x.id === id);
  if (!p) return;
  const settings = $<HTMLElement>("settings");
  if (p.area === "main") {
    lastMain = id;
    settings.hidden = true;
    showPage("main", id);
    $<HTMLElement>("main").scrollTop = 0;
  } else {
    lastSettings = id;
    settings.hidden = false;
    showPage("settings", id);
    $<HTMLElement>("settings_body").scrollTop = 0;
  }
  markNav();
});

function closeSettings(): void {
  router.go(lastMain);
}

function go(page: string): void {
  router.go(page === "settings" ? lastSettings : page);
}

// ------------------------------------------------------------ saving

let saveTimer = 0;
let statusTimer = 0;

/** A prefs field changed: save every prefs field the settings pages show. */
async function savePrefs(): Promise<void> {
  if (!prefsStore.get()) return;
  // AI last: Verbatim switches Auto-format (a general field) off.
  const patch: Partial<Prefs> = { ...collectGeneral(), ...collectAi() };
  try {
    await prefsStore.save(patch);
    setStatus("status", "Saved", true);
  } catch (e) {
    setStatus("status", String(e), false);
  }
  clearTimeout(statusTimer);
  statusTimer = setTimeout(() => ($<HTMLElement>("status").textContent = ""), 4000);
}

// ------------------------------------------------------------ build gates

/** data-needs: local / remote = an engine in this build, cloud = OpenRouter, dev = engine settings loaded. */
function applyNeeds(): void {
  const caps = views.dev?.caps;
  const has: Record<string, boolean> = {
    local: !!caps?.local,
    remote: !!caps?.remote,
    cloud: !!views.or?.supported,
    dev: !!views.dev,
  };
  for (const e of document.querySelectorAll<HTMLElement>("[data-needs]")) e.hidden = !has[e.dataset.needs ?? ""];
  $<HTMLElement>("tab_server").hidden = !caps?.remote;
  $<HTMLElement>("tab_openrouter").hidden = !views.or || (!views.or.supported && !views.or.notice);
  $<HTMLElement>("nav_developer").hidden = !views.dev?.visible;
  $<HTMLElement>("nav_connections").hidden =
    $<HTMLElement>("tab_server").hidden && $<HTMLElement>("tab_openrouter").hidden;
}

// ------------------------------------------------------------ boot

buildNav();
mountPages();
applyNeeds();

for (const b of document.querySelectorAll<HTMLButtonElement>(".lw-sidebar [data-page], #settings_nav [data-page]")) {
  b.addEventListener("click", () => go(b.dataset.page ?? "home"));
}
$<HTMLButtonElement>("settings_close").addEventListener("click", closeSettings);
$<HTMLElement>("settings").addEventListener("click", (e) => {
  if (e.target === e.currentTarget) closeSettings();
});
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || $<HTMLElement>("settings").hidden || onboarding.isOpen()) return;
  if (document.querySelector("dialog[open]")) return; // the dialog closes first
  closeSettings();
});
document.addEventListener("change", (e) => {
  const t = e.target as HTMLElement;
  if (t.dataset?.save !== "prefs") return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void savePrefs(), 30);
});
// Links between pages inside templates.
document.addEventListener("click", (e) => {
  const t = (e.target as HTMLElement).closest<HTMLElement>("[data-go]");
  if (t) router.go(t.dataset.go ?? "");
});

$<HTMLElement>("version_row").addEventListener("click", versionTapped);
views.onDev((v) => {
  $<HTMLElement>("version_row").textContent = `Murmur ${v.version}`;
  applyNeeds();
});
views.onOr((v) => {
  $<HTMLElement>("or_badge").hidden = !v?.notice;
  applyNeeds();
});
void onEvent<string>("settings:tab", (e) => go(OLD_TABS[e.payload] ?? e.payload));

async function load(): Promise<void> {
  await prefsStore.load();
  for (const f of [...FEATURES, enginesCard]) {
    try {
      await f.load?.();
    } catch (e) {
      console.error("load failed", e);
    }
  }
}

router.onNavigate((id, prev) => {
  for (const f of FEATURES) f.shown?.(id, id !== prev);
});

// First-run setup: over the shell until finished; Settings → General can
// run it again. Finishing reloads every page's state from the backend.
onboarding.init(() => void load());
document.addEventListener("click", (e) => {
  if (!(e.target as HTMLElement).closest("[data-run-setup]")) return;
  closeSettings();
  void onboarding.open();
});

router.go("home");
void load().then(() => {
  if (prefsStore.get()?.onboarded === false) void onboarding.open();
});
