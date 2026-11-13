// Loose transcript comparison: normalise, split into words, score by word
// edit distance (1 - WER, floored at 0). Punctuation, case, Hebrew niqqud
// and spacing do not count; the AI cleanup may rephrase, so callers pass
// with a threshold rather than demanding equality.

/** Lowercase, drop niqqud/cantillation and punctuation, collapse spaces. */
export function normalize(s) {
  return (s ?? "")
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[֑-ׇ]/g, "") // Hebrew points and accents
    .replace(/[‎‏‪-‮⁦-⁩]/g, "") // bidi marks
    .replace(/[^\p{L}\p{N}\s']/gu, " ")
    .replace(/'/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

export function words(s) {
  const n = normalize(s);
  return n ? n.split(" ") : [];
}

/** Levenshtein distance over word arrays. */
export function wordDistance(a, b) {
  const prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    let diag = prev[0];
    prev[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const up = prev[j];
      prev[j] = Math.min(prev[j] + 1, prev[j - 1] + 1, diag + (a[i - 1] === b[j - 1] ? 0 : 1));
      diag = up;
    }
  }
  return prev[b.length];
}

/**
 * 1 - word error rate of `got` against `want`, in [0, 1]. Both empty = 1.
 * Also returns the share of `want` words that appear anywhere in `got`
 * (recall), which forgives reordering by the cleanup.
 */
export function similarity(want, got) {
  const w = words(want);
  const g = words(got);
  if (!w.length) return { score: g.length ? 0 : 1, recall: g.length ? 0 : 1, wer: g.length ? 1 : 0 };
  const wer = wordDistance(w, g) / w.length;
  const pool = new Map();
  for (const x of g) pool.set(x, (pool.get(x) ?? 0) + 1);
  let hit = 0;
  for (const x of w) {
    if (pool.get(x)) {
      hit++;
      pool.set(x, pool.get(x) - 1);
    }
  }
  return { score: Math.max(0, 1 - wer), recall: hit / w.length, wer };
}

/** Share of letters in `s` that are Hebrew (0..1); null without letters. */
export function hebrewShare(s) {
  const letters = [...(s ?? "")].filter((c) => /\p{L}/u.test(c));
  if (!letters.length) return null;
  return letters.filter((c) => /[א-ת]/.test(c)).length / letters.length;
}
