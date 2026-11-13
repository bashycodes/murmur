// Probe (2026-11-13): does the laptop-side step marker catch an orphaned
// step? Runs sleep-step.ps1 as a step with a 5 s ssh timeout, so the ssh
// is killed while the step sleeps on; Remote then waits for it, stops it
// and reports the orphan. Then a dead-pid marker must read as stale.
// Touches nothing of the owner's. Usage (host from local.env):
//   node tools/probes/prototypes/windows_e2e_cdp_2026_11_13/stepguard-probe.mjs
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defaultHost } from "../../../e2e-windows/lib/args.mjs";
import { Remote, StepStopped } from "../../../e2e-windows/lib/remote.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const orphans = [];
const remote = new Remote({ host: defaultHost(), dir: "murmur-e2e", log: (s) => console.log(s), onOrphan: (m) => orphans.push(m) });
remote.sync(join(HERE, "../../../e2e-windows/ps"));
remote.scpTo([join(HERE, "sleep-step.ps1")], "");
try {
  remote.ps("sleep-step.ps1", ["-Sec", "120"], { timeoutMs: 5000 });
  console.log("UNEXPECTED: no timeout");
} catch (e) {
  console.log(e instanceof StepStopped ? "timed out as expected" : `other error: ${e.message}`);
}
console.log("orphans:", orphans);
console.log(remote.guardStep({ waitSec: 0 }));
// A leftover marker of a dead process reads as stale and is removed.
remote.ssh(`echo {"pid":4,"script":"x.ps1","proc_start":"2000-01-01T00:00:00.0000000Z"}> ${remote.winPath("step.json")}`);
console.log(remote.guardStep({ waitSec: 0 }));
console.log(remote.guardStep({ waitSec: 0 }));
remote.ssh(`del /q ${remote.winPath("sleep-step.ps1")}`);
remote.closeMaster();
