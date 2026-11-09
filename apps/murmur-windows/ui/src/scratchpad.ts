// Notes window (scratchpad.html). Two views of one page:
// - `?note=<id>` (or no query = the Quick note, id "scratchpad"): a
//   plain-text editor the user dictates into (the normal paste pipeline
//   types into this textarea like any other app), edits and copies out.
//   Every change goes to the shell (`note_edit`), which writes it to
//   %APPDATA%\Murmur\notes\<id>.txt after a short pause and on close/quit.
// - `?view=notes`: the Notes list (search by content, New note, Pin,
//   Export, Delete). Opening a note navigates this window to its editor.
// Styled with the app window's tokens (theme.css + components.css), light
// or dark like it. Wrapped in a function: tsc compiles all pages as one
// global scope.

(() => {
  const tauri = window.__TAURI__;

  // Light or dark: the .lw-dark class switches every theme.css token
  // (ui/src/lib/theme.ts does the same for the app window).
  const dark = window.matchMedia("(prefers-color-scheme: dark)");
  const applyTheme = (): void => {
    document.documentElement.classList.toggle("lw-dark", dark.matches);
  };
  applyTheme();
  dark.addEventListener("change", applyTheme);
  const params = new URLSearchParams(location.search);
  const QUICK = "scratchpad";

  interface NoteMeta {
    id: string;
    title: string;
    preview: string;
    words: number;
    created_ms: number;
    updated_ms: number;
    pinned: boolean;
    quick: boolean;
  }

  const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

  function plural(n: number, one: string): string {
    return n === 1 ? `1 ${one}` : `${n} ${one}s`;
  }

  function wordCount(text: string): number {
    return text.split(/\s+/).filter((w) => w.length > 0).length;
  }

  /** "14:05" today, "Mon 3 Jun" this year, else "3 Jun 2025". */
  function when(ms: number): string {
    if (!ms) return "";
    const d = new Date(ms);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) {
      return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    }
    const opts: Intl.DateTimeFormatOptions =
      d.getFullYear() === now.getFullYear()
        ? { weekday: "short", day: "numeric", month: "short" }
        : { day: "numeric", month: "short", year: "numeric" };
    return d.toLocaleDateString(undefined, opts);
  }

  function open(query: string): void {
    location.href = `scratchpad.html?${query}`;
  }

  /** A status line that clears itself after an ok message. */
  function statusLine(el: HTMLElement): (msg: string, kind?: "ok" | "err" | "") => void {
    let timer: number | undefined;
    return (msg, kind = "") => {
      el.textContent = msg;
      el.className = kind;
      window.clearTimeout(timer);
      if (kind === "ok") {
        timer = window.setTimeout(() => {
          el.textContent = "";
          el.className = "";
        }, 2500);
      }
    };
  }

  // ------------------------------------------------------------ list view

  function listView(): void {
    $("list-view").hidden = false;
    document.title = "Notes — Murmur";
    const search = $<HTMLInputElement>("search");
    const list = $<HTMLUListElement>("notes");
    const empty = $("empty");
    const count = $("count");
    const say = statusLine($("list-status"));
    let searchTimer: number | undefined;
    /** Note id whose Delete was clicked once (a second click deletes). */
    let armed: string | null = null;
    let armTimer: number | undefined;

    async function refresh(): Promise<void> {
      let notes: NoteMeta[];
      try {
        notes = await tauri.core.invoke<NoteMeta[]>("note_list", { query: search.value });
      } catch (e) {
        say(`Could not list the notes: ${e}`, "err");
        return;
      }
      list.replaceChildren(...notes.map(row));
      empty.hidden = notes.length > 0;
      count.textContent = search.value.trim() ? `${plural(notes.length, "match")}` : plural(notes.length, "note");
    }

    function button(text: string, cls: string, title: string, onClick: () => void): HTMLButtonElement {
      const b = document.createElement("button");
      b.type = "button";
      b.className = cls;
      b.textContent = text;
      b.title = title;
      b.addEventListener("click", onClick);
      return b;
    }

    function row(n: NoteMeta): HTMLLIElement {
      const li = document.createElement("li");
      li.dataset.id = n.id;
      if (n.pinned) li.classList.add("pinned");

      const openBtn = button("", "open", "Open", () => open(`note=${encodeURIComponent(n.id)}`));
      const title = document.createElement("span");
      title.className = "title";
      title.textContent = n.title;
      if (n.quick) {
        const badge = document.createElement("span");
        badge.className = "fl-badge fl-badge-light badge";
        badge.textContent = "Scratchpad";
        title.append(badge);
      }
      const meta = document.createElement("span");
      meta.className = "meta muted";
      const bits = [when(n.updated_ms), plural(n.words, "word")].filter((s) => s);
      if (n.quick && n.preview) bits.push(n.preview);
      meta.textContent = bits.join(" · ");
      openBtn.append(title, meta);

      const pin = button(n.pinned ? "Unpin" : "Pin", "fl-btn fl-btn-ghost fl-btn-sm pin", n.pinned ? "Unpin" : "Pin to the top", async () => {
        try {
          await tauri.core.invoke("note_set_pinned", { id: n.id, pinned: !n.pinned });
        } catch (e) {
          say(`${e}`, "err");
        }
        await refresh();
      });

      const exp = document.createElement("select");
      exp.className = "fl-select export";
      exp.title = "Save a copy to Downloads";
      exp.setAttribute("aria-label", "Export");
      for (const [v, t] of [["", "Export…"], ["txt", ".txt"], ["md", ".md"]]) {
        const o = document.createElement("option");
        o.value = v;
        o.textContent = t;
        exp.append(o);
      }
      exp.addEventListener("change", async () => {
        const ext = exp.value;
        exp.value = "";
        if (!ext) return;
        try {
          const path = await tauri.core.invoke<string>("note_export", { id: n.id, ext });
          say(`Saved ${path}`, "ok");
        } catch (e) {
          say(`Export failed: ${e}`, "err");
        }
      });

      li.append(openBtn, pin, exp);
      if (!n.quick) {
        const del = button("Delete", "fl-btn fl-btn-ghost fl-btn-sm lw-danger delete", "Delete this note", async () => {
          if (armed !== n.id) {
            armed = n.id;
            del.textContent = "Delete?";
            window.clearTimeout(armTimer);
            armTimer = window.setTimeout(() => {
              armed = null;
              del.textContent = "Delete";
            }, 3000);
            say("Click Delete? again to delete the note");
            return;
          }
          armed = null;
          try {
            await tauri.core.invoke("note_delete", { id: n.id });
            say("Deleted", "ok");
          } catch (e) {
            say(`Delete failed: ${e}`, "err");
          }
          await refresh();
        });
        li.append(del);
      }
      return li;
    }

    search.addEventListener("input", () => {
      window.clearTimeout(searchTimer);
      searchTimer = window.setTimeout(() => void refresh(), 150);
    });

    $("new-note").addEventListener("click", async () => {
      try {
        const id = await tauri.core.invoke<string>("note_create");
        open(`note=${encodeURIComponent(id)}`);
      } catch (e) {
        say(`Could not create a note: ${e}`, "err");
      }
    });

    void refresh();
    search.focus();
  }

  // ---------------------------------------------------------- editor view

  function editorView(noteId: string): void {
    $("editor-view").hidden = false;
    const editor = $<HTMLTextAreaElement>("editor");
    const words = $("words");
    const name = $("note-name");
    const copyBtn = $<HTMLButtonElement>("copy");
    const clearBtn = $<HTMLButtonElement>("clear");
    const undoBtn = $<HTMLButtonElement>("undo");
    const say = statusLine($("status"));

    /** Text before the last Clear, until the next edit. */
    let undoText: string | null = null;

    function showCount(): void {
      words.textContent = plural(wordCount(editor.value), "word");
      if (noteId === QUICK) {
        name.textContent = "Quick note";
      } else {
        const first = editor.value.split("\n").map((l) => l.trim()).find((l) => l) ?? "";
        name.textContent = first || "Untitled";
      }
      document.title = `${name.textContent} — Murmur`;
    }

    function setUndo(text: string | null): void {
      undoText = text;
      undoBtn.hidden = text === null;
    }

    async function send(): Promise<void> {
      showCount();
      try {
        await tauri.core.invoke("note_edit", { id: noteId, text: editor.value });
      } catch (e) {
        say(`Not saved: ${e}`, "err");
      }
    }

    editor.addEventListener("input", () => {
      setUndo(null);
      void send();
    });

    copyBtn.addEventListener("click", async () => {
      try {
        await tauri.core.invoke("clipboard_copy", { text: editor.value });
        say("Copied", "ok");
      } catch (e) {
        say(`Copy failed: ${e}`, "err");
      }
      editor.focus();
    });

    clearBtn.addEventListener("click", () => {
      if (editor.value === "") return;
      setUndo(editor.value);
      editor.value = "";
      void send();
      say("Cleared");
      editor.focus();
    });

    undoBtn.addEventListener("click", () => {
      if (undoText === null) return;
      editor.value = undoText;
      setUndo(null);
      void send();
      say("");
      editor.focus();
    });

    for (const ext of ["txt", "md"]) {
      $(`export-${ext}`).addEventListener("click", async () => {
        try {
          const path = await tauri.core.invoke<string>("note_export", { id: noteId, ext });
          say(`Saved ${path}`, "ok");
        } catch (e) {
          say(`Export failed: ${e}`, "err");
        }
        editor.focus();
      });
    }

    $("back").addEventListener("click", () => open("view=notes"));

    // Closing the window or leaving for the list: write now rather than
    // after the debounce (the shell also flushes on window close and quit).
    window.addEventListener("pagehide", () => {
      void tauri.core.invoke("note_flush", { id: noteId });
    });

    async function init(): Promise<void> {
      try {
        editor.value = await tauri.core.invoke<string>("note_load", { id: noteId });
      } catch (e) {
        // Don't let typing here overwrite a file we could not read.
        editor.readOnly = true;
        clearBtn.disabled = true;
        say(`Could not open the note: ${e}`, "err");
      }
      showCount();
      editor.focus();
      editor.setSelectionRange(editor.value.length, editor.value.length);
    }

    void init();
  }

  if (params.get("view") === "notes") {
    listView();
  } else {
    editorView(params.get("note") || QUICK);
  }
})();
