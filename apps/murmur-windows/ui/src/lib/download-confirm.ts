// What the download confirm dialog (D31) shows for a catalogue entry.
// Pure, so tests/download-confirm.test.mjs can run it; features/models.ts
// fills the dialog with it.

/** The language-ID model (D34), the Hebrew models' only extra today. */
export const LID_FILE = "ggml-tiny-q5_1.bin";

/** The catalogue fields the dialog needs (features/models.ts CatalogEntry). */
export interface ConfirmEntry {
  title: string;
  page: string;
  dest: string;
  size: number;
  installed: boolean;
  complete?: boolean;
  missing?: number;
  from_server?: boolean;
  files: { name: string; url: string; extra?: boolean }[];
}

export interface ConfirmFacts {
  title: string;
  /** A URL, or the server's name when it comes from the server's library. */
  from: string;
  size: number;
  files: number;
  dest: string;
}

/** The folder a Windows or POSIX path sits in (the path itself when it has none). */
export function dirname(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut > 0 ? path.slice(0, cut) : path;
}

/**
 * The dialog's facts. A model installed without its extras (a Hebrew model
 * downloaded before the language-ID model came with it) fetches only those,
 * into the models folder, so the dialog names and sizes just them.
 */
export function confirmFacts(e: ConfirmEntry, serverName: string): ConfirmFacts {
  const extras = e.files.filter((f) => f.extra);
  const extrasOnly = e.installed && e.complete === false && extras.length > 0 && !!e.missing;
  if (extrasOnly) {
    const lid = extras.every((f) => f.name === LID_FILE);
    return {
      title: lid ? "the language-detection model" : extras.map((f) => f.name).join(", "),
      from: e.from_server ? serverName : extras[0].url,
      size: e.missing ?? 0,
      files: extras.length,
      dest: dirname(e.dest),
    };
  }
  const from = e.from_server ? serverName : e.files.length > 1 ? e.page : e.files[0].url;
  return { title: e.title, from, size: e.size, files: e.files.length, dest: e.dest };
}
