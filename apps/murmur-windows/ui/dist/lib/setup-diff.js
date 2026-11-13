// Re-running setup (Settings → General → Run setup): what the saved settings
// say for each step's choice, and the one line before Finish that says what
// Finish leaves changed ("Cleanup: Your server → Rules only") with Keep /
// Change (UX review on 1.3.0: a re-run used to replace a working setup
// silently, D95). Pure, so `npm test` runs it on the built dist without a
// page.
const WHAT = {
    engine: "Speech",
    model: "Speech model",
    cleanup: "Cleanup",
    ptt: "Shortcut",
    mic: "Microphone",
};
const ENGINE = { pc: "This PC", server: "Your server", cloud: "Cloud" };
const CLEANUP = { rules: "Rules only", pc: "On this PC", server: "Your server", cloud: "Cloud" };
/** A choice in setup's own words. */
export function choiceLabel(key, c) {
    switch (key) {
        case "engine":
            return c.engine ? ENGINE[c.engine] : "Not set up";
        case "model":
            return c.model ?? "None";
        case "cleanup":
            return CLEANUP[c.cleanup];
        case "ptt":
            return c.ptt.replace(/\+/g, " + ");
        case "mic":
            return c.mic || "System default";
    }
}
const ORDER = ["engine", "model", "cleanup", "ptt", "mic"];
/** What differs between the settings setup opened with and the ones it would finish with, in step order. */
export function setupChanges(before, after) {
    return ORDER.filter((k) => choiceLabel(k, before) !== choiceLabel(k, after)).map((k) => ({
        key: k,
        what: WHAT[k],
        from: choiceLabel(k, before),
        to: choiceLabel(k, after),
    }));
}
/** The line before Finish. */
export function changesLine(changes) {
    if (!changes.length)
        return "Nothing changes. Finish keeps your setup as it was.";
    return `Finish changes ${changes.map((c) => `${c.what}: ${c.from} → ${c.to}`).join("; ")}.`;
}
/** The step where a choice is made (Change goes there). */
export function stepOf(key) {
    return key === "engine" ? "engine" : key === "model" ? "model" : key === "cleanup" ? "cleanup" : "try";
}
const ENGINE_OF = { client: "pc", server: "server", cloud: "cloud" };
/** The first engine the saved settings would use: on, and (a server) with an address. */
export function savedEngine(f) {
    const first = f.route_order.find((r) => r === "client" ? f.allow_client : r === "server" ? f.allow_server && !!f.host.trim() : f.cloudStt);
    return first ? ENGINE_OF[first] : null;
}
