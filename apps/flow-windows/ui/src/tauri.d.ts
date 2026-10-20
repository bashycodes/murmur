// Minimal typing for the global Tauri API (app.withGlobalTauri = true), so
// the front-end needs no bundler and no npm runtime dependencies.
interface TauriEvent<T> {
  payload: T;
}

interface Window {
  __TAURI__: {
    core: {
      invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
    };
    event: {
      listen<T>(event: string, cb: (e: TauriEvent<T>) => void): Promise<() => void>;
    };
  };
}
