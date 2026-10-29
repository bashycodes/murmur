// History: the local SQLite transcripts (copy, re-paste, delete, undo AI
// edit = use the raw ASR text instead of the cleaned one) and retention.

import { $, button, el, html, setStatus, when } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";

interface HistoryItem {
  id: number;
  ts: number;
  app: string;
  url: string | null;
  raw: string;
  formatted: string | null;
  pasted: string | null;
  edited: string | null;
  duration_ms: number;
  words: number;
  lang: string | null;
  final_text: string;
  ai_edited: boolean;
}

const TEMPLATE = `
<section class="panel" data-panel="history">
  <fieldset>
    <legend>History</legend>
    <div class="toolbar">
      <input id="hist_query" type="search" placeholder="Search transcripts" spellcheck="false" />
      <button type="button" id="hist_clear" class="danger">Delete all</button>
    </div>
    <label>Keep transcripts on this PC
      <select id="retention">
        <option value="keep">Forever</option>
        <option value="day">24 hours</option>
        <option value="never">Never (nothing is stored)</option>
      </select>
    </label>
    <p class="hint">Text only, in a local SQLite file. Audio is never stored. <span id="hist_status"></span></p>
  </fieldset>
  <ol id="hist_list" class="hist"></ol>
  <p id="hist_empty" class="hint" hidden>No transcripts.</p>
</section>`;

/** Word-level LCS diff: [kind, word] with kind "=", "-" (raw only), "+" (final only). */
function wordDiff(a: string, b: string): [string, string][] {
  const x = a.split(/\s+/).filter(Boolean);
  const y = b.split(/\s+/).filter(Boolean);
  const n = x.length;
  const m = y.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: [string, string][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      out.push(["=", x[i]]);
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) out.push(["-", x[i++]]);
    else out.push(["+", y[j++]]);
  }
  while (i < n) out.push(["-", x[i++]]);
  while (j < m) out.push(["+", y[j++]]);
  return out;
}

function diffView(raw: string, final: string): HTMLElement {
  const p = el("p", "diff");
  for (const [kind, w] of wordDiff(raw, final)) {
    const tag = kind === "-" ? "del" : kind === "+" ? "ins" : "span";
    p.append(el(tag, undefined, w), " ");
  }
  return p;
}

async function act(what: Promise<unknown>, okText: string): Promise<void> {
  try {
    await what;
    setStatus("hist_status", okText, true);
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

function historyRow(h: HistoryItem): HTMLLIElement {
  const li = el("li");
  const meta = el("div", "meta");
  meta.append(el("span", undefined, when(h.ts)));
  if (h.app) meta.append(el("span", undefined, h.app.replace(/\.exe$/i, "")));
  meta.append(el("span", undefined, `${h.words} words`));
  if (h.ai_edited) meta.append(el("span", "chip", "AI edited"));
  if (h.edited) meta.append(el("span", "chip", "you corrected it"));
  if (h.pasted === null) meta.append(el("span", "chip err", "not pasted"));
  li.append(meta, el("p", "text", h.final_text));

  const actions = el("div", "actions");
  actions.append(
    button("Copy", () => void act(invoke("history_copy", { id: h.id, raw: false }), "Copied")),
    button("Re-paste", () => void act(invoke("history_repaste", { id: h.id, raw: false }), "Pasted")),
  );
  if (h.ai_edited) {
    const undo = el("div", "undo");
    undo.hidden = true;
    undo.append(
      el("p", "hint", "What you said (struck = removed by AI, underlined = added):"),
      diffView(h.raw, h.final_text),
      button("Re-paste original", () =>
        void act(invoke("history_repaste", { id: h.id, raw: true }), "Pasted the original"),
      ),
      button("Copy original", () =>
        void act(invoke("history_copy", { id: h.id, raw: true }), "Copied the original"),
      ),
    );
    actions.append(
      button("Undo AI edit…", () => {
        undo.hidden = !undo.hidden;
      }),
    );
    li.append(actions, undo);
  } else {
    li.append(actions);
  }
  if (h.edited) {
    const c = el("p", "hint");
    c.append("Your correction: ", el("q", undefined, h.edited));
    li.append(c);
  }
  actions.append(
    button(
      "Delete",
      () =>
        void (async () => {
          try {
            await invoke("history_delete", { id: h.id });
            li.remove();
          } catch (e) {
            setStatus("hist_status", String(e), false);
          }
        })(),
      "small danger",
    ),
  );
  return li;
}

let histSeq = 0;
async function loadHistory(): Promise<void> {
  const seq = ++histSeq;
  try {
    const rows = await invoke<HistoryItem[]>("history_list", {
      query: $<HTMLInputElement>("hist_query").value,
      limit: 200,
    });
    if (seq !== histSeq) return; // a newer search is in flight
    const list = $<HTMLOListElement>("hist_list");
    list.replaceChildren(...rows.map(historyRow));
    $<HTMLElement>("hist_empty").hidden = rows.length > 0;
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

let clearArmed = 0;
async function clearHistory(): Promise<void> {
  const b = $<HTMLButtonElement>("hist_clear");
  if (Date.now() - clearArmed > 3000) {
    clearArmed = Date.now();
    b.textContent = "Click again to delete all";
    setTimeout(() => (b.textContent = "Delete all"), 3000);
    return;
  }
  clearArmed = 0;
  b.textContent = "Delete all";
  await act(invoke("history_clear"), "History deleted");
  await loadHistory();
}

async function saveRetention(): Promise<void> {
  try {
    await prefsStore.save({ retention: $<HTMLSelectElement>("retention").value });
    setStatus("hist_status", "Saved", true);
    await loadHistory();
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

export const historyPage = {
  id: "history",
  mount(root: HTMLElement): void {
    root.append(html(TEMPLATE));
    let searchTimer = 0;
    $<HTMLInputElement>("hist_query").addEventListener("input", () => {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => void loadHistory(), 200);
    });
    $<HTMLButtonElement>("hist_clear").addEventListener("click", () => void clearHistory());
    $<HTMLSelectElement>("retention").addEventListener("change", () => void saveRetention());
    void onEvent<null>("history:changed", () => {
      if (!$<HTMLElement>("hist_list").closest("[hidden]")) void loadHistory();
    });
  },
  async load(): Promise<void> {
    const p = prefsStore.get();
    if (p) $<HTMLSelectElement>("retention").value = p.retention;
  },
  show(): void {
    void loadHistory();
  },
};
