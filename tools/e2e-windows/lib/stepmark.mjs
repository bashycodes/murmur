// Laptop steps and their marker (ps/run-step.ps1, ps/stepguard.ps1): the
// pure parts, so they can be tested here.
//
// Every state-changing script runs through run-step.ps1, which writes
// murmur-e2e\step.json {pid, script, start} first and removes it in its own
// finally. A marker whose pid is still alive is a step still running on the
// laptop, possibly orphaned (its local ssh was killed by a timeout, a
// SIGKILL or a forced Ctrl+C). Before a new step, a restore or breaking a
// lock, such a step is waited for (bounded), then stopped with its process
// tree.

/** Read-only scripts / actions: no marker, no guard (they must stay quick). */
const READ_ONLY = {
  "lock.ps1": true,
  "stepguard.ps1": true,
  "appdata.ps1": ["status", "ownerprefs"],
  "vbcable.ps1": ["status"],
  "audio.ps1": ["list", "waveout"],
  "uninstall-data.ps1": ["check"],
};

/** Does `script args` change anything on the laptop (so it gets a marker)? */
export function isStateChanging(script, args = []) {
  const ro = READ_ONLY[script];
  if (ro === true) return false;
  if (script === "install.ps1" && args.includes("-VersionOnly")) return false;
  if (Array.isArray(ro)) {
    const i = args.indexOf("-Action");
    return !(i >= 0 && ro.includes(args[i + 1]));
  }
  return true;
}

/**
 * ["-Action", "restore", "-Keep", "-File", "x"] -> {Action: "restore", Keep: true, File: "x"}:
 * the named parameters run-step.ps1 splats into the script. A flag with no
 * value (end, or followed by another -Name) is a switch.
 */
export function toNamedParams(args) {
  const out = {};
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (!/^-[A-Za-z]\w*$/.test(a)) throw new Error(`run-step: positional argument ${JSON.stringify(a)} (use -Name value)`);
    const next = args[i + 1];
    if (next === undefined || /^-[A-Za-z]\w*$/.test(next)) out[a.slice(1)] = true;
    else {
      out[a.slice(1)] = next;
      i++;
    }
  }
  return out;
}

/** The base64 (UTF-8 JSON) argument run-step.ps1 decodes. */
export function encodeParams(args) {
  return Buffer.from(JSON.stringify(toNamedParams(args)), "utf8").toString("base64");
}

/**
 * The guard's report (stepguard.ps1 or run-step.ps1 output):
 * step_guard=none|finished|stale|killed|alive, step_script=, step_pid=, step_waited_s=.
 * Returns null when there is no report, else {state, script, pid, waited, orphan}.
 * `orphan` = a step was still running and had to be stopped (a clean-up error).
 */
export function parseGuard(out) {
  const kv = {};
  for (const line of String(out).split(/\r?\n/)) {
    const m = /^(step_guard|step_script|step_pid|step_waited_s)=(.*)$/.exec(line.trim());
    if (m) kv[m[1]] = m[2].trim();
  }
  if (!kv.step_guard) return null;
  return {
    state: kv.step_guard,
    script: kv.step_script || null,
    pid: kv.step_pid ? Number(kv.step_pid) : null,
    waited: kv.step_waited_s ? Number(kv.step_waited_s) : 0,
    orphan: kv.step_guard === "killed" || kv.step_guard === "alive",
  };
}

/** A second Ctrl+C within this many ms forces the running step to stop. */
export const FORCE_WINDOW_MS = 3000;

/**
 * What a signal means, given the time of the previous one (null = first):
 * "cleanup" (first: clean up, letting the laptop step finish), "force"
 * (second within 3 s: stop the laptop step, then clean up), "ignore".
 */
export function signalAction(prevAt, now, forced) {
  if (prevAt === null) return "cleanup";
  if (!forced && now - prevAt <= FORCE_WINDOW_MS) return "force";
  return "ignore";
}
