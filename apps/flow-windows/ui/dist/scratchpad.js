"use strict";
// Scratchpad window: one plain-text note the user dictates into (the normal
// paste pipeline types into this textarea like any other app), edits and
// copies out. Every change goes to the shell (`note_edit`), which writes it
// to %APPDATA%\FlowClone\<id>.txt after a short pause and on close/quit.
// The note id comes from the URL (`scratchpad.html?note=<id>`, default
// "scratchpad") so a later Notes list can reuse this page per note.
// Wrapped in a function: tsc compiles all pages as one global scope.
(() => {
    const tauri = window.__TAURI__;
    const noteId = new URLSearchParams(location.search).get("note") || "scratchpad";
    const editor = document.getElementById("editor");
    const words = document.getElementById("words");
    const statusEl = document.getElementById("status");
    const copyBtn = document.getElementById("copy");
    const clearBtn = document.getElementById("clear");
    const undoBtn = document.getElementById("undo");
    /** Text before the last Clear, until the next edit. */
    let undoText = null;
    let statusTimer;
    function wordCount(text) {
        return text.split(/\s+/).filter((w) => w.length > 0).length;
    }
    function showCount() {
        const n = wordCount(editor.value);
        words.textContent = n === 1 ? "1 word" : `${n} words`;
    }
    function say(msg, kind = "") {
        statusEl.textContent = msg;
        statusEl.className = kind;
        window.clearTimeout(statusTimer);
        if (kind === "ok") {
            statusTimer = window.setTimeout(() => {
                statusEl.textContent = "";
                statusEl.className = "";
            }, 2000);
        }
    }
    function setUndo(text) {
        undoText = text;
        undoBtn.hidden = text === null;
    }
    async function send() {
        showCount();
        try {
            await tauri.core.invoke("note_edit", { id: noteId, text: editor.value });
        }
        catch (e) {
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
        }
        catch (e) {
            say(`Copy failed: ${e}`, "err");
        }
        editor.focus();
    });
    clearBtn.addEventListener("click", () => {
        if (editor.value === "")
            return;
        setUndo(editor.value);
        editor.value = "";
        void send();
        say("Cleared");
        editor.focus();
    });
    undoBtn.addEventListener("click", () => {
        if (undoText === null)
            return;
        editor.value = undoText;
        setUndo(null);
        void send();
        say("");
        editor.focus();
    });
    // Closing the window: write now rather than after the debounce (the shell
    // also flushes on window close and on quit).
    window.addEventListener("pagehide", () => {
        void tauri.core.invoke("note_flush", { id: noteId });
    });
    async function init() {
        try {
            editor.value = await tauri.core.invoke("note_load", { id: noteId });
        }
        catch (e) {
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
})();
