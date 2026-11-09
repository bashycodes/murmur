// Browser stand-in for window.__TAURI__ on ui/dist/scratchpad.html (probe
// only, nothing here ships). Notes live in localStorage under "mock-note:<id>"
// plus "mock-meta" (pinned/created/updated), written on every
// note_edit/flush (no debounce: that, the index and search are tested in
// Rust, src/notes.rs); every invoke is recorded in window.__mockCalls,
// exports in window.__exports.
(() => {
  const key = (id) => `mock-note:${id}`;
  const QUICK = "scratchpad";
  const validId = (id) => /^[a-z0-9_-]{1,64}$/.test(id);
  const meta = () => JSON.parse(localStorage.getItem("mock-meta") || "{}");
  const setMeta = (m) => localStorage.setItem("mock-meta", JSON.stringify(m));
  const exists = (id) => id === QUICK || localStorage.getItem(key(id)) !== null;
  const check = (id) => {
    if (!validId(id)) throw `"${id}" is not a note id`;
  };
  let clock = Number(localStorage.getItem("mock-clock") || Date.now());
  const tick = () => {
    clock += 1000;
    localStorage.setItem("mock-clock", String(clock));
    return clock;
  };
  const save = (id, text) => {
    localStorage.setItem(key(id), text);
    const m = meta();
    const now = tick();
    m[id] = { pinned: false, created_ms: now, ...m[id], updated_ms: now };
    setMeta(m);
  };
  const flushAll = () => {
    for (const id of Object.keys(window.__pending)) {
      save(id, window.__pending[id]);
      delete window.__pending[id];
    }
  };
  const firstLine = (t) => t.split("\n").map((l) => l.trim()).find((l) => l) || "";
  window.__mockCalls = [];
  window.__clipboard = null;
  window.__pending = {};
  window.__exports = [];
  const cmds = {
    note_load: ({ id }) => {
      check(id);
      return window.__pending[id] ?? localStorage.getItem(key(id)) ?? "";
    },
    note_edit: ({ id, text }) => {
      check(id);
      window.__pending[id] = text;
    },
    note_flush: ({ id }) => {
      if (id in window.__pending) {
        save(id, window.__pending[id]);
        delete window.__pending[id];
      }
    },
    note_list: ({ query }) => {
      flushAll();
      const ids = new Set([QUICK]);
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (k.startsWith("mock-note:")) ids.add(k.slice(10));
      }
      const m = meta();
      const q = (query || "").trim().toLowerCase();
      return [...ids]
        .map((id) => {
          const text = localStorage.getItem(key(id)) ?? "";
          const e = m[id] || { pinned: false, created_ms: 0, updated_ms: 0 };
          const line = firstLine(text);
          const quick = id === QUICK;
          return {
            id,
            text,
            title: quick ? "Quick note" : line || "Untitled",
            preview: quick ? line : "",
            words: text.split(/\s+/).filter((w) => w).length,
            created_ms: e.created_ms,
            updated_ms: e.updated_ms,
            pinned: e.pinned,
            quick,
          };
        })
        .filter((n) => !q || n.text.toLowerCase().includes(q))
        .sort((a, b) => b.pinned - a.pinned || b.updated_ms - a.updated_ms || (a.id < b.id ? -1 : 1))
        .map(({ text, ...n }) => n);
    },
    note_create: () => {
      let id = `note-${tick()}`;
      save(id, "");
      return id;
    },
    note_delete: ({ id }) => {
      check(id);
      if (id === QUICK) throw "the Quick note cannot be deleted (use Clear)";
      delete window.__pending[id];
      localStorage.removeItem(key(id));
      const m = meta();
      delete m[id];
      setMeta(m);
    },
    note_set_pinned: ({ id, pinned }) => {
      check(id);
      if (!exists(id)) throw `no note "${id}"`;
      const m = meta();
      m[id] = { created_ms: 0, updated_ms: 0, ...m[id], pinned };
      setMeta(m);
    },
    note_export: ({ id, ext }) => {
      check(id);
      cmds.note_flush({ id });
      const text = localStorage.getItem(key(id)) ?? "";
      const path = `C:\\Users\\me\\Downloads\\${firstLine(text) || "Quick note"}.${ext}`;
      window.__exports.push([path, text]);
      return path;
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
