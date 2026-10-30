// Which page is on screen. Every page renders its own static template once
// at boot into an empty container the shell made for it; the router only
// shows one page and tells it so.

export interface Page {
  id: string;
  /** "main" = the panel next to the sidebar; "settings" = the Settings window. */
  area: "main" | "settings";
  /** The page's static markup (never user text). */
  template: string;
  /** Wire the page's controls. Runs once every template is in the DOM. */
  init?(): void;
  /** The page came on screen; `opened` = it was not on screen before. */
  show?(opened: boolean): void;
}

/** Logic shared by several pages (it finds its controls by id in their
 *  templates): wired after every template is in the DOM, loaded at boot. */
export interface Feature {
  init(): void;
  load?(): Promise<void>;
  /** Called after every navigation with the page now on screen. */
  shown?(page: string, opened: boolean): void;
}

const pages = new Map<string, Page>();
let active = "";
const navListeners: ((id: string, prev: string) => void)[] = [];

export const router = {
  register(p: Page): void {
    pages.set(p.id, p);
  },
  pages(): Page[] {
    return [...pages.values()];
  },
  has(id: string): boolean {
    return pages.has(id);
  },
  active(): string {
    return active;
  },
  /** Called after every navigation (the shell shows the page, marks its nav). */
  onNavigate(l: (id: string, prev: string) => void): void {
    navListeners.push(l);
  },
  go(id: string): void {
    if (!pages.has(id)) return;
    const prev = active;
    const opened = id !== prev;
    active = id;
    for (const l of navListeners) l(id, prev);
    pages.get(id)?.show?.(opened);
  },
};
