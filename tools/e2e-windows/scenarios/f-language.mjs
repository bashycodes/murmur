// (f) Language detection: a Hebrew clip and an English clip, dictated with
// no language pinned (seed: languages [] and keyboard_language off). The
// language comes from the debug log (language, language_source) and the
// history entry; the text from the pad.
// Pass = he clip detected as Hebrew with mostly-Hebrew text, en clip as
// English. The mixed clip is reported, not graded.
import { dictateIntoPad, sleep } from "../lib/steps.mjs";
import { hebrewShare } from "../lib/similarity.mjs";

/**
 * pass = every graded try ok; fail = a graded try that ran was WRONG (even
 * if another was aborted: a wrong language is a finding, an abort is not);
 * aborted = the only misses are aborted tries.
 */
export function languageStatus(tries) {
  const graded = tries.filter((t) => t.graded);
  if (graded.every((t) => t.ok)) return "pass";
  if (graded.some((t) => !t.aborted && !t.ok)) return "fail";
  return "aborted";
}

const CASES = [
  { clip: "he-shop", want: "he", graded: true },
  { clip: "en-short", want: "en", graded: true },
  { clip: "mixed-meeting", want: null, graded: false },
];

export default {
  id: "f",
  title: "Language detection (Hebrew / English)",
  needs: "seeded",
  async run(ctx) {
    const { app } = ctx;
    const tries = [];
    const evidence = [];
    for (const c of CASES) {
      const r = await dictateIntoPad(ctx, c.clip, `f-${c.clip}`);
      if (r.aborted) {
        tries.push({ ...c, aborted: true, aborted_at: r.aborted_at, ok: c.graded ? false : null });
        evidence.push(`${c.clip}: ABORTED ${r.aborted_at}`);
        continue;
      }
      await sleep(500);
      let historyError = null;
      const h = await app.invoke("history_list", { query: "", limit: 1 }).catch((e) => {
        historyError = String(e?.message ?? e);
        return [];
      });
      const entry = Array.isArray(h) ? h[0] : (h?.items ?? h?.entries ?? [])[0];
      if (!entry && !historyError) historyError = "history_list returned no rows";
      if (historyError) ctx.log(`f ${c.clip}: history: ${historyError}`);
      const lang = r.debug?.language ?? entry?.lang ?? entry?.language ?? null;
      const he = hebrewShare(r.got ?? "") ?? 0; // no letters = nothing typed
      const ok = c.want === "he" ? /^he|^iw/i.test(lang ?? "") && he >= 0.5 : c.want === "en" ? /^en/i.test(lang ?? "") && he < 0.2 : null;
      tries.push({ ...c, language: lang, language_source: r.debug?.language_source ?? null, history_lang: entry?.lang ?? null, history_error: historyError, history_route: entry?.route ?? null, debug_stop_language: r.debug?.stop_language ?? null, debug_route: r.debug?.route ?? null, debug_insert: r.debug?.insert ?? null, debug_text: r.debug?.text ?? null, hebrew_share: he, got: r.got, score: r.verdict.score, ok, shot: r.shot });
      evidence.push(`${c.clip}: language=${lang} (source ${r.debug?.language_source ?? "?"}), Hebrew share ${he.toFixed(2)}, score ${r.verdict.score.toFixed(2)}, got "${r.got ?? ""}"; ${r.shot}`);
    }
    return { status: languageStatus(tries), summary: tries.map((t) => t.aborted ? `${t.clip}→ABORTED` : `${t.clip}→${t.language ?? "?"}${t.graded ? (t.ok ? " ok" : " WRONG") : ""}`).join(", "), details: { tries }, evidence };
  },
};
