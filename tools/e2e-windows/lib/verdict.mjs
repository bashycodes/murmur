// The run's overall verdict and exit code (run.sh documents the codes).

export const EXIT = {
  ok: 0, // every scenario passed and the owner's data is back
  runError: 1, // the run itself failed (setup, laptop, harness)
  usage: 2, // bad arguments
  failed: 3, // a scenario failed / errored, or a clean-up step failed, or the restore did not happen
  busy: 4, // --no-wait and another run holds the laptop (or the wait timed out)
  noRestore: 5, // --no-restore: the test install's data is still live; results.pass has the verdict
  interrupted: 130, // SIGINT / SIGTERM: clean-up ran, the run did not finish
};

/**
 * Did the run pass? Every scenario passed, no run error, no clean-up error,
 * and (unless --no-restore) the owner's data was positively restored.
 * @param {object} results  results.json
 * @param {{restore: boolean}} args
 */
export function computePass(results, args) {
  const scenarios = Object.values(results.scenarios ?? {});
  if (results.error || results.interrupted) return false;
  if (!scenarios.length || !scenarios.every((s) => s.status === "pass")) return false;
  if (results.cleanup_errors?.length) return false;
  if (args.restore && results.restored !== true) return false;
  return true;
}

/** Exit code for a finished (or interrupted) run. */
export function exitCode(results, args) {
  if (results.interrupted) return EXIT.interrupted;
  if (results.error) return EXIT.runError;
  if (!args.restore) return EXIT.noRestore;
  return results.pass ? EXIT.ok : EXIT.failed;
}

/** Compare dotted versions ("1.3.0" vs "1.10.2"): <0, 0, >0. */
export function compareVersions(a, b) {
  const pa = String(a).split(/[.+-]/).map((x) => parseInt(x, 10) || 0);
  const pb = String(b).split(/[.+-]/).map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

/** Version in an installer's file name (Murmur_1.3.0_x64-setup_hybrid.exe → "1.3.0"), or null. */
export function installerVersion(name) {
  return /(?:^|[_\-\s])(\d+\.\d+\.\d+)(?:[_\-.]|$)/.exec(String(name).replace(/^.*[\\/]/, ""))?.[1] ?? null;
}
