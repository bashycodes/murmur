// (c) Hold-to-talk: hold Right Alt over a clip with our text pad focused; the
// text the pad ends up with is read back through UI Automation.
// Pass = every clip's transcript matches loosely (word level).
import { dictateIntoPad } from "../lib/steps.mjs";

const CLIPS = ["en-short", "en-report"];

export default {
  id: "c",
  title: "Hold-to-talk into a text box",
  needs: "seeded",
  async run(ctx) {
    const tries = [];
    const evidence = [];
    for (const id of CLIPS) {
      const r = await dictateIntoPad(ctx, id, `c-${id}`);
      tries.push(r);
      if (r.aborted) {
        evidence.push(`${id}: ABORTED ${r.aborted_at}`);
        continue;
      }
      evidence.push(`${id}: score ${r.verdict.score.toFixed(2)} wer ${r.verdict.wer.toFixed(2)}; got "${r.got ?? ""}"; route ${r.debug?.route ?? "?"}; ${r.shot}`);
    }
    const ok = tries.filter((t) => t.verdict.pass).length;
    const aborted = tries.filter((t) => t.aborted).length;
    const status = ok === tries.length ? "pass" : aborted && ok + aborted === tries.length ? "aborted" : "fail";
    return { status, summary: `${ok}/${tries.length} clips transcribed into the pad${aborted ? `; ${aborted} aborted (pad lost the foreground)` : ""}`, details: { tries }, evidence };
  },
};
