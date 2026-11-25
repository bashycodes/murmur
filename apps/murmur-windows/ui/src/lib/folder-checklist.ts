// "Use model files on this PC" (setup) and "Import from folder…" (Settings
// → Speech to text): the checklist of what a picked folder holds for the
// three things local dictation in English and Hebrew needs. The backend
// scans (`model_folder_scan`, src/model_import.rs) and imports
// (`model_folder_import`); this decides what each row says and offers.
// Pure, so tests/folder-checklist.test.mjs can run it.

export type RowKey = "english" | "hebrew" | "lid";
export const ROWS: RowKey[] = ["english", "hebrew", "lid"];

export const ROW_LABEL: Record<RowKey, string> = {
  english: "English speech model",
  hebrew: "Hebrew speech model",
  lid: "Language auto-detect model",
};

/** `model_import::SlotView`. */
export type SlotView =
  | { state: "found"; id: string; title: string; size: number; folder: string; installed: boolean }
  | { state: "missing" }
  | { state: "problem"; title: string; message: string };

/** `model_import::FolderScan`. */
export interface FolderScan {
  folder: string;
  english: SlotView;
  hebrew: SlotView;
  lid: SlotView;
  others: { id: string; title: string; size: number; kind: "speech" | "hebrew" | "cleanup" }[];
  problems: { title: string; message: string }[];
  unrecognised: { path: string; size: number; why: string }[];
  /** Too big or too deep to search whole: what's past the limit wasn't looked at. */
  large: boolean;
}

/** `model_import::SlotResult`. */
export type SlotResult = { state: "skipped" } | { state: "imported"; id: string; title: string } | { state: "failed"; message: string };

/** `model_import::FolderImportView`. */
export interface FolderImportView {
  english: SlotResult;
  hebrew: SlotResult;
  lid: SlotResult;
  auto_detect: boolean;
  cancelled: boolean;
}

/** `model_import::FolderPick`. */
export type FolderPick = Record<RowKey, boolean>;

/** The catalogue fields a row's Download needs (features/models.ts CatalogEntry). */
export interface DlEntry {
  id: string;
  title: string;
  size: number;
  language?: string;
  engine: string;
  recommended: boolean;
  installed: boolean;
  complete?: boolean;
  missing?: number;
}

/** What a missing row would download, and how big it is. */
export interface DlTarget {
  id: string;
  size: number;
  /** The button's text. */
  label: string;
}

/**
 * The model a missing row downloads: the recommended English model, the
 * recommended Hebrew model, and for language detection the installed Hebrew
 * model's missing extra (D31: a download of an installed model fetches only
 * what's missing) or, with no Hebrew model yet, the recommended one, which
 * brings it along.
 */
export function downloadTarget(key: RowKey, catalog: DlEntry[]): DlTarget | undefined {
  const speech = catalog.filter((c) => c.engine !== "llm");
  const mb = (n: number): string => `${Math.max(1, Math.round(n / 1e6))} MB`;
  if (key === "english") {
    const e = speech.find((c) => !c.language && c.recommended);
    return e && { id: e.id, size: e.size, label: `Download ${mb(e.size)}` };
  }
  const he = speech.find((c) => !!c.language && c.recommended);
  if (key === "hebrew") return he && { id: he.id, size: he.size, label: `Download ${mb(he.size)}` };
  const own = speech.find((c) => !!c.language && c.installed && c.complete === false && !!c.missing);
  if (own) return { id: own.id, size: own.missing ?? 0, label: `Download ${mb(own.missing ?? 0)}` };
  return he && { id: he.id, size: he.size, label: `Download with ${he.title} (${mb(he.size)})` };
}

/** One row's facts, besides the scan. */
export interface RowInput {
  slot: SlotView;
  /** After "Use these" (absent before). */
  result?: SlotResult;
  /** A found row is ticked (the default). */
  use: boolean;
  /** Skip was pressed on a row that has nothing to use. */
  skipped: boolean;
  /** The model is already usable on this PC (a download or an earlier import). */
  have: boolean;
  /** Its download: what Download fetches (absent: this build can't), and whether one runs. */
  download?: DlTarget;
  canStart: boolean;
  /** Percent of this row's running download, else null. */
  pct: number | null;
}

export type Tone = "ok" | "warn" | "err" | "muted";

export interface RowView {
  name: string;
  detail: string;
  badge: string;
  tone: Tone;
  /** Show the Use tick box (a found row before "Use these"). */
  tick: boolean;
  /** Show Download (with this label) / Skip. */
  download: string | null;
  skip: boolean;
}

const SETTINGS_HINT = "You can add it later in Settings → Speech to text.";

export function mbText(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

/** What a row shows. */
export function rowView(i: RowInput): RowView {
  const fix = (detail: string, badge: string, tone: Tone, name: string): RowView => {
    if (i.pct !== null) return { name, detail: "Downloading…", badge: `Downloading ${i.pct}%`, tone: "muted", tick: false, download: null, skip: false };
    if (i.have) return { name, detail: "Ready on this PC.", badge: "Ready", tone: "ok", tick: false, download: null, skip: false };
    if (i.skipped) return { name, detail: `Skipped. ${SETTINGS_HINT}`, badge: "Skipped", tone: "muted", tick: false, download: null, skip: false };
    const dl = i.download && i.canStart ? i.download.label : null;
    return { name, detail, badge, tone, tick: false, download: dl, skip: true };
  };
  const r = i.result;
  if (r?.state === "imported") return { name: r.title, detail: "Copied and checked. Ready.", badge: "Ready", tone: "ok", tick: false, download: null, skip: false };
  if (r?.state === "failed") return fix(r.message, "Not imported", "err", i.slot.state === "found" ? i.slot.title : "Not imported");
  const s = i.slot;
  if (s.state === "found") {
    const where = s.folder ? ` · in ${s.folder}` : "";
    const already = s.installed ? " · already on this PC" : "";
    if (r?.state === "skipped" || (r === undefined && !i.use)) {
      if (r === undefined) return { name: s.title, detail: `${mbText(s.size)}${where} · won't be used`, badge: "Found", tone: "muted", tick: true, download: null, skip: false };
      return fix(`Not used. ${SETTINGS_HINT}`, "Not used", "muted", s.title);
    }
    return { name: s.title, detail: `${mbText(s.size)}${where}${already}`, badge: "Found", tone: "ok", tick: r === undefined, download: null, skip: false };
  }
  if (s.state === "problem") return fix(s.message, "Can't use", "warn", s.title);
  const how = i.download && i.canStart ? "Download it, or skip it." : `Skip it. ${SETTINGS_HINT}`;
  return fix(`Not found in this folder. ${how}`, "Not found", "muted", "Not found");
}

/** The rows to import: found and ticked. */
export function pickOf(scan: FolderScan, use: Record<RowKey, boolean>): FolderPick {
  const on = (k: RowKey): boolean => scan[k].state === "found" && use[k];
  return { english: on("english"), hebrew: on("hebrew"), lid: on("lid") };
}

/** "Use these" can be pressed: at least one found row is ticked. */
export function canUse(scan: FolderScan | null, use: Record<RowKey, boolean>): boolean {
  if (!scan) return false;
  const p = pickOf(scan, use);
  return p.english || p.hebrew || p.lid;
}

/** The line under the rows after a scan: nothing usable, or how many found. */
export function scanLine(scan: FolderScan): string {
  const n = ROWS.filter((k) => scan[k].state === "found").length;
  if (n === 3) return "All three are here. Use these to set up English, Hebrew and auto-detect.";
  if (n === 0) return "None of the three models is in this folder. Choose another folder, or download them.";
  return `Found ${n} of 3. What's missing can be downloaded or skipped.`;
}

/** Everything in the folder that none of the rows uses: listed, never hidden. */
export function notes(scan: FolderScan): string[] {
  const out: string[] = [];
  if (scan.large) out.push("This folder is large, so only part of it was searched. Pick the folder that holds the models.");
  for (const o of scan.others) {
    const where = o.kind === "cleanup" ? "Settings → AI cleanup" : "Settings → Speech to text";
    out.push(`Also here, not used: ${o.title} (${mbText(o.size)}). Import it in ${where}.`);
  }
  for (const p of scan.problems) out.push(`${p.title}: ${p.message}.`);
  for (const f of scan.unrecognised) out.push(`${f.path} (${mbText(f.size)}): ${f.why}.`);
  return out;
}

/** The line while a scan reads a file to tell which model it is. */
export function checkingLine(file: string | null): string {
  return file ? `Checking ${file}…` : "Looking through the folder…";
}

/** The line after "Use these": what's set up now. */
export function importLine(v: FolderImportView): { text: string; ok: boolean } {
  if (v.cancelled) return { text: "Import cancelled. Models already copied stay set up.", ok: false };
  const failed = ROWS.filter((k) => v[k].state === "failed").length;
  const done = ROWS.filter((k) => v[k].state === "imported").length;
  if (failed && !done) return { text: "Nothing was imported. Each row says why.", ok: false };
  const lang = v.auto_detect ? " Language is now Automatic: speak English or Hebrew." : "";
  if (failed) return { text: `Imported ${done} of ${done + failed}.${lang} The others say why.`, ok: false };
  return { text: `Done. The models are copied and checked.${lang}`, ok: true };
}
