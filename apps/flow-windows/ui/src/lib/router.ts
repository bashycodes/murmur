// Which page is on screen. Every page renders its own DOM once at boot
// into its own container; the router only shows one and tells it so.

export interface Page {
  id: string;
  /** The nav entry that shows this page (default: its id). */
  tab?: string;
  /** Render this page's DOM into `root` and wire its controls. */
  mount(root: HTMLElement): void;
  /** Fetch the page's data at boot (after every page is mounted). */
  load?(): Promise<void>;
  /** The page came on screen; `opened` = it was not on screen before. */
  show?(opened: boolean): void;
}

const pages = new Map<string, Page>();
let active = "";
const navListeners: ((id: string) => void)[] = [];

export const router = {
  register(p: Page): void {
    pages.set(p.id, p);
  },
  pages(): Page[] {
    return [...pages.values()];
  },
  active(): string {
    return active;
  },
  /** Called after every navigation (the shell marks its nav). */
  onNavigate(l: (id: string) => void): void {
    navListeners.push(l);
  },
  go(id: string): void {
    const opened = id !== active;
    active = id;
    for (const l of navListeners) l(id);
    for (const p of pages.values()) if ((p.tab ?? p.id) === id) p.show?.(opened);
  },
};
