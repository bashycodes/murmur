// The last DevView (`dev_get` / `dev_set`) and OrView (`dev_or_*`) the page
// received, shared by every page that shows engine or cloud state. The
// page that fetched a view publishes it; others subscribe instead of
// reaching into each other.

import type { DevView, OrView } from "./types.js";

type DevListener = (v: DevView) => void;
type OrListener = (v: OrView | null, prev: OrView | null) => void;

const devListeners: DevListener[] = [];
const orListeners: OrListener[] = [];

export const views = {
  dev: null as DevView | null,
  or: null as OrView | null,

  onDev(l: DevListener): void {
    devListeners.push(l);
  },
  onOr(l: OrListener): void {
    orListeners.push(l);
  },

  setDev(v: DevView): void {
    views.dev = v;
    for (const l of devListeners) l(v);
  },
  /** null = a build without OpenRouter. */
  setOr(v: OrView | null): void {
    const prev = views.or;
    views.or = v;
    for (const l of orListeners) l(v, prev);
  },
};
