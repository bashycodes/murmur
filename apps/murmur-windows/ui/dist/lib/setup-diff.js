// Re-running setup (Settings → General → Run setup): what the saved settings
// say for each choice setup can change, and the one line before Finish that
// says what Finish leaves changed ("Cleanup: Your server → Rules only") with
// Keep / Change (UX review on 1.3.0: a re-run used to replace a working
// setup silently, D95). Choices are compared as saved (paths, addresses),
// never by their display names; names are only for the line. Pure, so
// `npm test` runs it on the built dist without a page.
import { cloudShortName } from "./provider-names.js";
import { ENGINE_LABEL } from "./setup-engine.js";
const WHAT = {
    engine: "Speech",
    host: "Server address",
    token: "Server token",
    model: "Speech model",
    hebrew: "Hebrew model",
    lid: "Language detection model",
    language: "Language",
    source: "Download from",
    cleanup: "Cleanup",
    llmPath: "Cleanup model",
    ptt: "Shortcut",
    mic: "Microphone",
};
const CLEANUP = { rules: "Rules only", pc: "On this PC", server: "Your server", cloud: "Cloud" };
const SOURCE = { server: "Your server", huggingface: "Hugging Face" };
const fileName = (p) => p.split(/[\\/]/).pop() || p;
const sameModel = (a, b) => a === b ||
    (!!a &&
        !!b &&
        a.asr_engine === b.asr_engine &&
        // Only the engine's own file matters: Parakeet keeps a Whisper path set, and the other way round.
        (a.asr_engine === "parakeet" ? (a.parakeet_model_dir ?? "") === (b.parakeet_model_dir ?? "") : (a.model_path ?? "") === (b.model_path ?? "")));
const sameHebrew = (a, b) => (a.hebrew?.model ?? "") === (b.hebrew?.model ?? "") && (a.hebrew?.dir ?? null) === (b.hebrew?.dir ?? null);
const languageKey = (c) => (c.language?.keyboard ? "keyboard" : (c.language?.codes ?? []).join(","));
/** Each choice: whether two configs differ, and how one reads. */
const CHOICES = {
    engine: { same: (a, b) => a.engine === b.engine, label: (c, l) => (c.engine === "cloud" ? cloudShortName(l.provider) : c.engine ? ENGINE_LABEL[c.engine] : "Not set up") },
    host: { same: (a, b) => a.host.trim() === b.host.trim(), label: (c) => c.host.trim() || "None" },
    token: { same: (a, b) => a.tokenSaves === b.tokenSaves, label: (c) => (c.tokenSaves ? "Replaced" : "As it was") },
    model: { same: (a, b) => sameModel(a.model, b.model), label: (c, l) => (c.model ? l.modelName(c.model) : "None") },
    hebrew: {
        same: sameHebrew,
        label: (c, l) => (c.hebrew?.dir ? fileName(c.hebrew.dir) : c.hebrew?.model ? (l.hebrewName?.(c.hebrew.model) ?? c.hebrew.model) : "None"),
    },
    lid: { same: (a, b) => (a.lid ?? null) === (b.lid ?? null), label: (c) => (c.lid ? fileName(c.lid) : "In the models folder") },
    language: {
        same: (a, b) => languageKey(a) === languageKey(b),
        label: (c, l) => {
            if (c.language?.keyboard)
                return "Keyboard language";
            const codes = c.language?.codes ?? [];
            return codes.length ? (l.languageNames?.(codes) ?? codes).join(", ") : "Automatic";
        },
    },
    source: { same: (a, b) => a.source === b.source, label: (c) => SOURCE[c.source] ?? c.source },
    cleanup: { same: (a, b) => a.cleanup === b.cleanup, label: (c, l) => (c.cleanup === "cloud" ? cloudShortName(l.provider) : CLEANUP[c.cleanup]) },
    llmPath: { same: (a, b) => (a.llmPath ?? "") === (b.llmPath ?? ""), label: (c) => (c.llmPath ? fileName(c.llmPath) : "None") },
    ptt: { same: (a, b) => a.ptt === b.ptt, label: (c) => c.ptt.replace(/\+/g, " + ") },
    mic: { same: (a, b) => (a.mic || "") === (b.mic || ""), label: (c) => c.mic || "System default" },
};
const ORDER = ["engine", "host", "token", "model", "hebrew", "lid", "language", "source", "cleanup", "llmPath", "ptt", "mic"];
/** A choice in setup's own words. */
export function choiceLabel(key, c, l) {
    return CHOICES[key].label(c, l);
}
/** What differs between the settings setup opened with and the ones it would finish with, in step order. */
export function setupChanges(before, after, l) {
    return ORDER.filter((k) => !CHOICES[k].same(before, after)).map((k) => ({
        key: k,
        what: WHAT[k],
        from: choiceLabel(k, before, l),
        to: choiceLabel(k, after, l),
    }));
}
/** The line before Finish. A new token reads "Server token replaced" (its value is never shown). */
export function changesLine(changes) {
    if (!changes.length)
        return "Nothing changes. Finish keeps your setup as it was.";
    const part = (c) => (c.key === "token" ? "Server token replaced" : `${c.what}: ${c.from} → ${c.to}`);
    return `Finish changes ${changes.map(part).join("; ")}.`;
}
/** The step where a choice is made (Change goes there). "Download from" is on step 4. */
export const STEP_OF = {
    engine: "engine",
    host: "engine",
    token: "engine",
    model: "model",
    hebrew: "model",
    lid: "model",
    language: "model",
    source: "cleanup",
    cleanup: "cleanup",
    llmPath: "cleanup",
    ptt: "try",
    mic: "try",
};
const ENGINE_OF = { client: "pc", server: "server", cloud: "cloud" };
/** The first engine the saved settings would use: on, and (a server) with an address. */
export function savedEngine(f) {
    const first = f.route_order.find((r) => r === "client" ? f.allow_client : r === "server" ? f.allow_server && !!f.host.trim() : f.cloudStt);
    return first ? ENGINE_OF[first] : null;
}
