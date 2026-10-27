// Browser stand-in for window.__TAURI__ on ui/dist/scratchpad.html (probe
// only, nothing here ships). Notes live in localStorage under "mock-note:<id>",
// written on every note_edit/flush (no debounce: that is tested in Rust,
// src/notes.rs); every invoke is recorded in window.__mockCalls.
(() => {
  const key = (id) => `mock-note:${id}`;
  window.__mockCalls = [];
  window.__clipboard = null;
  window.__pending = {};
  const cmds = {
    note_load: ({ id }) => window.__pending[id] ?? localStorage.getItem(key(id)) ?? "",
    note_edit: ({ id, text }) => {
      if (!/^[a-z0-9_-]{1,64}$/.test(id)) throw `"${id}" is not a note id`;
      window.__pending[id] = text;
    },
    note_flush: ({ id }) => {
      if (id in window.__pending) {
        localStorage.setItem(key(id), window.__pending[id]);
        delete window.__pending[id];
      }
    },
    clipboard_copy: ({ text }) => {
      window.__clipboard = text;
    },
  };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args = {}) => {
        window.__mockCalls.push([cmd, args]);
        if (!(cmd in cmds)) throw `mock: ${cmd} is not allowed in the Scratchpad`;
        return cmds[cmd](args);
      },
    },
    event: { listen: async () => () => {} },
  };
})();
