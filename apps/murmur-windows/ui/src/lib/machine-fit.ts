// This PC against the machine-class table (D117): the `machine_fit` answer
// and the words setup and Settings show for it. The table itself lives in
// murmur-core (`model_fit`); this file only words its answer. Pure, so
// `npm test` runs it on the built dist without a page.

export interface FitView {
  fits: boolean;
  /** "needs 2.2 GB free", "needs 8 GB RAM", "needs a graphics chip". */
  reasons: string[];
}

/** `machine_fit` (src/machine_fit.rs MachineView). */
export interface MachineFit {
  /** Bytes; 0 = not known. */
  total_ram: number;
  available_ram?: number;
  free_disk?: number;
  /** Local AI cleanup would run on a GPU. */
  gpu: boolean;
  gpu_device?: string;
  whisper_gpu: boolean;
  /** Catalogue ids the table picks. */
  speech: string;
  /** null: no local AI cleanup recommended on this PC. */
  cleanup: string | null;
  hebrew: string;
  /** Every catalogue model, by id. */
  fit: Record<string, FitView>;
}

/** "Fits this PC", "Doesn't fit: needs 8 GB RAM", or "" when nothing is known about the model. */
export function fitText(m: MachineFit | null | undefined, id: string | null | undefined): string {
  const f = id ? m?.fit[id] : undefined;
  if (!f) return "";
  return f.fits ? "Fits this PC" : `Doesn't fit: ${f.reasons.join(", ")}`;
}

/** Whether the model is known not to fit (unknown: fits, never blocks). */
export function doesNotFit(m: MachineFit | null | undefined, id: string | null | undefined): boolean {
  const f = id ? m?.fit[id] : undefined;
  return !!f && !f.fits;
}

/** The warning under a picked model that doesn't fit: it is still allowed. */
export function fitWarning(m: MachineFit | null | undefined, id: string | null | undefined, title: string): string {
  if (!doesNotFit(m, id)) return "";
  const f = m?.fit[id ?? ""];
  return `${title} may not run well on this PC (${(f?.reasons ?? []).join(", ")}). You can still use it.`;
}

const gb = (b: number): string => `${Math.round(b / 2 ** 30)} GB`;

/** "This PC: 16 GB RAM (9 GB free) · 120 GB free for models · graphics chip: …"; "" when nothing is known. */
export function machineLine(m: MachineFit | null | undefined): string {
  if (!m) return "";
  const parts: string[] = [];
  if (m.total_ram > 0) parts.push(`${gb(m.total_ram)} RAM${m.available_ram !== undefined ? ` (${gb(m.available_ram)} free)` : ""}`);
  if (m.free_disk !== undefined) parts.push(`${Math.floor(m.free_disk / 1e9)} GB free for models`);
  parts.push(m.gpu && m.gpu_device ? `graphics chip: ${m.gpu_device}` : "no graphics chip for AI");
  return `This PC: ${parts.join(" · ")}`;
}

/** Setup's two speech rows: the one the table picks (the Whisper row stands for every whisper model). */
export function recommendedRow(m: MachineFit | null | undefined, parakeetIds: string[]): "parakeet" | "whisper" {
  if (!m) return "parakeet";
  return parakeetIds.includes(m.speech) ? "parakeet" : "whisper";
}
