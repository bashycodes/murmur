// D52: which cleanup provider the Cloud server card shows. Picking a
// provider is saved by `dev_or_set_custom_cleanup`, but "Custom server" is
// only saved once its address and model are typed, and a switch can wait on
// a consent dialog. Until then the pick lives here, so a re-render from the
// saved settings (e.g. `loadOr` after the key's Save) does not flip the
// selector back to the saved provider. No DOM: unit-tested under ui/tests.

import type { CleanupProvider } from "./types.js";

export class ProviderChoice {
  private pending: CleanupProvider | null = null;
  private inFlight: Promise<unknown> = Promise.resolve();

  /** The provider the card shows while `saved` is the stored one. */
  shown(saved: CleanupProvider): CleanupProvider {
    return this.pending ?? saved;
  }

  /** A pick is waiting to be saved: the card's fields hold the user's unsaved input. */
  get picking(): boolean {
    return this.pending !== null;
  }

  /** The user picked `p` (not saved yet); picking the saved one again drops the pick. */
  pick(p: CleanupProvider, saved: CleanupProvider | undefined): void {
    this.pending = p === saved ? null : p;
  }

  /** The pick was saved, or given up (consent refused): the stored provider shows again. */
  settle(): void {
    this.pending = null;
  }

  /** Run `save` after any save already running, so a later reload sees its result. */
  run<T>(save: () => Promise<T>): Promise<T> {
    const next = this.inFlight.then(save, save);
    this.inFlight = next.catch(() => undefined);
    return next;
  }

  /** Resolves once every save started so far is done. */
  idle(): Promise<void> {
    return this.inFlight.then(() => undefined);
  }
}
