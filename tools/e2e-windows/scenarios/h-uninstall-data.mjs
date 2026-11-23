// (h) The uninstaller's "Delete the application data" box removes the app's
// own data folder (%APPDATA%\Murmur) as well as Tauri's %LOCALAPPDATA%\dev.murmur.windows; unticked, or when an
// installer runs over an existing install, nothing is deleted.
//
// The box is only on the uninstaller's confirm page, which /S never shows,
// so both uninstalls run the uninstaller in the interactive session and
// drive the page through Win32 messages (agent `nsis_uninstall`).
//
// Steps (dummy data only: the owner's is in this run's backup, and the
// seed refuses otherwise):
//   1. the run's install + seed  -> uninstall, box ticked   -> all gone
//   2. install + seed            -> uninstall, box unticked -> it stays
//   3. install over that data (fresh install), then the same installer
//      again over the install with /S, then with /P /UPDATE (as the
//      updater runs it)                                     -> it stays
// The test build stays installed; the harness then restores the owner's data.
// Runs last: it uninstalls and reinstalls the app under the other scenarios.
import { join } from "node:path";
import { Remote } from "../lib/remote.mjs";

const REG_KEY = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Murmur";

/** What each check expects (`check` output of uninstall-data.ps1). */
export const EXPECT = {
  seeded: { murmur: "present" },
  ticked: { murmur: "absent", local: "absent", installed: "False" },
  unticked: { murmur: "present", installed: "False" },
  installed: { murmur: "present", installed: "True" },
  "silent-over": { murmur: "present", installed: "True" },
  update: { murmur: "present", installed: "True" },
};

/** Mismatches between a check's key=value map and what `phase` expects. */
export function gradePhase(phase, kv) {
  const want = EXPECT[phase];
  if (!want) throw new Error(`unknown phase ${phase}`);
  return Object.entries(want)
    .filter(([k, v]) => kv[k] !== v)
    .map(([k, v]) => `${phase}: ${k}=${kv[k] ?? "?"}, want ${v}`);
}

/** Mismatches in the uninstaller dialog: the box as asked, and the entry gone. */
export function gradeDialog(phase, d) {
  const out = [];
  const want = phase === "ticked" ? "On" : "Off";
  if (d.checkbox_before !== "Off") out.push(`${phase}: the box started ${d.checkbox_before}, want Off (the default)`);
  if (d.checkbox_after !== want) out.push(`${phase}: the box was ${d.checkbox_after} when uninstalling, want ${want}`);
  if (!d.entry_removed) out.push(`${phase}: the uninstall entry is still there`);
  return out;
}

/** Full Windows path of the installer for a process started by the agent. */
export function installerPath(installer) {
  return /[\\/:]/.test(installer) ? installer : `%USERPROFILE%\\Downloads\\${installer}`;
}

export default {
  id: "h",
  title: "Uninstall deletes app data only when asked",
  async run(ctx) {
    const { app, remote, agent, log, results } = ctx;
    const evidence = [];
    const problems = [];
    const details = { checks: {}, dialogs: {}, installs: {} };
    const check = (phase) => {
      const kv = Remote.kv(remote.ps("uninstall-data.ps1", ["-Action", "check"]).out);
      details.checks[phase] = kv;
      const bad = gradePhase(phase, kv);
      problems.push(...bad);
      evidence.push(`${phase}: Murmur ${kv.murmur}, Local\\dev.murmur.windows ${kv.local}, installed ${kv.installed}${bad.length ? " (UNEXPECTED)" : ""}`);
      return kv;
    };
    const seed = () => {
      app.stop();
      log(remote.ps("uninstall-data.ps1", ["-Action", "seed", "-Id", results.run]).out.trim());
      return check("seeded");
    };
    const install = (what) => {
      app.stop();
      const out = remote.ps("install.ps1", ["-Installer", ctx.installer], { timeoutMs: 300_000 }).out;
      const kv = Remote.kv(out);
      details.installs[what] = { version: kv.version, exit: kv.installer_exit };
      if (kv.exe) app.exe = kv.exe;
      app.stop(); // the uninstaller refuses while the app runs
    };
    const uninstall = async (phase) => {
      app.stop();
      const { uninstaller } = Remote.kv(remote.ps("uninstall-data.ps1", ["-Action", "check"]).out);
      if (!uninstaller) throw new Error(`${phase}: Murmur is not installed`);
      const shot = `h-confirm-${phase}`;
      const d = await agent.call("nsis_uninstall", { uninstaller, delete_data: phase === "ticked", regKey: REG_KEY, timeoutSec: 180, shot: remote.winPath(`shots/${shot}.png`) }, 300_000);
      details.dialogs[phase] = d;
      problems.push(...gradeDialog(phase, d));
      if (d.shot) {
        try {
          remote.scpFrom(`shots/${shot}.png`, join(ctx.dir, "shots", `${shot}.png`));
          evidence.push(`${phase}: confirm page "${d.checkbox_name}" ${d.checkbox_after}: shots/${shot}.png`);
        } catch (e) {
          log(`shot ${shot}: ${e.message}`);
        }
      }
      return check(phase);
    };
    const over = async (phase, args) => {
      app.stop();
      const r = await agent.call("run_wait", { exe: installerPath(ctx.installer), args, timeoutSec: 300 }, 360_000);
      details.installs[phase] = { args, exit: r.exit_code };
      if (r.exit_code !== 0) problems.push(`${phase}: installer ${args.join(" ")} exited ${r.exit_code}`);
      app.stop();
      return check(phase);
    };

    // The owner's data must be out of the live folders before anything here
    // writes or deletes them.
    const pending = Remote.kv(remote.ps("appdata.ps1", ["-Action", "status"]).out).pending;
    if (pending !== results.run) return { status: "error", summary: `the owner's data is not in this run's backup (pending=${pending ?? "none"}): not touching the data folders`, details, evidence };

    // 1. Ticked.
    if (gradePhase("seeded", seed()).length) return { status: "error", summary: "could not seed the data folders", details, evidence };
    await uninstall("ticked");
    // 2. Unticked.
    install("reinstall");
    if (gradePhase("seeded", seed()).length) return { status: "error", summary: "could not seed the data folders", details, evidence };
    await uninstall("unticked");
    // 3. Installs over existing data and an existing install.
    install("over-data");
    check("installed");
    await over("silent-over", ["/S"]);
    await over("update", ["/P", "/UPDATE"]);

    const pass = problems.length === 0;
    if (!pass) evidence.push(...problems.map((p) => `FAIL ${p}`));
    const summary = pass
      ? "ticked: Murmur deleted; unticked, install over, /S over and /P /UPDATE: kept"
      : problems.join("; ");
    return { status: pass ? "pass" : "fail", summary, details, evidence };
  },
};
