// Setup's "Where should your voice become text?" step: which card carries
// "Recommended" and which one starts selected (D73, amended by the 1.3.0 UX
// review: step 1 promises "Runs on this PC", so a new install is pointed at
// it). Pure, so `npm test` runs it on the built dist without a page.

export type SetupEngine = "pc" | "server" | "cloud";

/**
 * The engine setup recommends: your server once one is saved (the user
 * already has one), else this PC. A build without either recommends nothing.
 */
export function recommendedEngine(has: (e: SetupEngine) => boolean, serverSaved: boolean): SetupEngine | null {
  if (has("server") && serverSaved) return "server";
  if (has("pc")) return "pc";
  return null;
}

/**
 * The card selected when the step opens. `first` is the first engine the saved
 * settings would use (null: none set up). A server or OpenRouter already in use
 * stays picked, and so does this PC once setup has been finished (re-run from
 * Settings). On a clean install this PC is on by default but was never chosen,
 * so the recommended engine is picked; a build with neither engine it could
 * recommend starts on your server, else the cloud.
 */
export function initialEngine(
  first: SetupEngine | null,
  onboarded: boolean,
  has: (e: SetupEngine) => boolean,
  serverSaved: boolean,
): SetupEngine {
  if (first === "server" || first === "cloud") return first;
  if (first === "pc" && onboarded) return "pc";
  return recommendedEngine(has, serverSaved) ?? (has("server") ? "server" : "cloud");
}
