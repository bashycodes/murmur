// summary.md of one run (results.json holds everything).
import { writeFileSync } from "node:fs";
import { join } from "node:path";

export function writeSummary(dir, r) {
  const lines = [
    `# Murmur Windows e2e — ${r.run}`,
    "",
    `- **Overall: ${r.pass ? "PASS" : "FAIL"}**`,
    `- Installer: \`${r.installer}\` (version ${r.version ?? "?"})`,
    `- Started ${r.started}, finished ${r.finished ?? "?"}`,
    `- Owner data restored: ${r.restored === undefined ? "n/a (no backup made)" : r.restored ? "yes" : "NO — run `run.sh restore`"}`,
    `- Owner's Murmur before the run: ${r.owner_version ?? "none"} (the test build stays installed)`,
  ];
  if (r.interrupted) lines.push(`- **Interrupted** by ${r.interrupted} (clean-up ran)`);
  if (r.error) lines.push(`- **Run error:** ${r.error}`);
  if (r.leftover_tasks) lines.push(`- **Leftover scheduled tasks:** ${r.leftover_tasks}`);
  if (r.cleanup_errors) lines.push(`- **Clean-up errors:** ${r.cleanup_errors.join("; ")}`);
  lines.push("", "| Scenario | Result | Summary |", "|---|---|---|");
  for (const [id, s] of Object.entries(r.scenarios)) {
    lines.push(`| ${id}. ${s.title} | **${s.status.toUpperCase()}** | ${String(s.summary).replaceAll("|", "\\|").replaceAll("\n", " ")} |`);
  }
  const rows = r.scenarios.d?.details?.rows;
  if (rows?.length) {
    lines.push("", "## d. Double-tap table", "", "| Profile | Key | Tap ms | Gap ms | Locked | Aborted | Out of 500 ms window | Press-to-press min/med/max | Graded |", "|---|---|---|---|---|---|---|---|---|");
    for (const x of rows) {
      lines.push(`| ${x.profile} | ${x.key}${x.bound ? "" : " (unbound)"} | ${x.tap_ms} | ${x.gap_ms} | ${x.locked}/${x.tries} | ${x.aborted ?? 0} | ${x.out_of_window} | ${x.press_to_press_ms.join("/")} | ${x.graded ? "yes" : "no"} |`);
    }
  }
  for (const [id, s] of Object.entries(r.scenarios)) {
    if (!s.evidence?.length) continue;
    lines.push("", `## ${id}. ${s.title}`, "");
    for (const e of s.evidence) lines.push(`- ${e}`);
  }
  writeFileSync(join(dir, "summary.md"), lines.join("\n") + "\n");
}
