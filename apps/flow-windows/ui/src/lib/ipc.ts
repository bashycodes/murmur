// The only door to the backend (`src/app.rs` commands + events). Pages use
// these instead of touching window.__TAURI__ so the mock (tools/probes/
// settings_mock/mock-tauri.js) and the real app look the same to them.

export function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return window.__TAURI__.core.invoke<T>(cmd, args);
}

export function onEvent<T>(name: string, cb: (e: TauriEvent<T>) => void): Promise<() => void> {
  return window.__TAURI__.event.listen<T>(name, cb);
}
