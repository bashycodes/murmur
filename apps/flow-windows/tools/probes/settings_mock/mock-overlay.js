// Browser stand-in for window.__TAURI__ in ui/dist/overlay.html. Probe only:
// window.__emit(event, payload) fires the backend events the overlay listens to;
// overlay_action calls are logged to window.__actions.
(() => {
  const handlers = {};
  window.__actions = [];
  window.__emit = (event, payload) => (handlers[event] || []).forEach((h) => h({ event, payload }));
  window.__TAURI__ = {
    event: {
      listen: async (event, h) => {
        (handlers[event] = handlers[event] || []).push(h);
        return () => {};
      },
    },
    core: {
      invoke: async (cmd, args) => {
        window.__actions.push({ cmd, args });
        return null;
      },
    },
  };
})();
