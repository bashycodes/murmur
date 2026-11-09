// The only door to the backend (`src/app.rs` commands + events). Pages use
// these instead of touching window.__TAURI__ so the mock (tools/probes/
// settings_mock/mock-tauri.js) and the real app look the same to them.
export function invoke(cmd, args) {
    return window.__TAURI__.core.invoke(cmd, args);
}
export function onEvent(name, cb) {
    return window.__TAURI__.event.listen(name, cb);
}
