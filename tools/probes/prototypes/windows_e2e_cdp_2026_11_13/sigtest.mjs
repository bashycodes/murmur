// Probe: is a SIGINT that lands during spawnSync(ssh) seen by node's handler,
// with and without an async ssh child (like the harness's tunnels) alive?
//   MURMUR_E2E_HOST=user@host timeout -s INT 3 node sigtest.mjs [tunnel]
import { spawn, spawnSync } from "node:child_process";
process.on("SIGINT", () => {
  console.error("HANDLER");
  process.exit(130);
});
const host = process.env.MURMUR_E2E_HOST;
if (process.argv[2] === "tunnel") {
  spawn("ssh", ["-o", "BatchMode=yes", "-N", "-L", "9299:127.0.0.1:9299", host], { stdio: "ignore" });
  await new Promise((res) => setTimeout(res, 1500));
}
const r = spawnSync("ssh", ["-o", "BatchMode=yes", host, "ping -n 6 127.0.0.1 >nul & echo done"], { encoding: "utf8" });
console.error("spawn", r.status, r.signal);
await new Promise((res) => setTimeout(res, 1000));
console.error("after timer");
