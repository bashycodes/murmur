// Scenarios in run order. `needs`: "fresh" = right after the fresh install
// (no prefs), "seeded" = the owner's prefs + models with the VB-Cable mic.
// The fresh ones must come first: seeding replaces the fresh profile.
import a from "./a-first-launch.mjs";
import b from "./b-onboarding.mjs";
import c from "./c-hold-to-talk.mjs";
import d from "./d-double-alt.mjs";
import e from "./e-scratchpad.mjs";
import f from "./f-language.mjs";
import g from "./g-unfinished-setup.mjs";
import h from "./h-uninstall-data.mjs";

// g restarts the fresh install with setup left half way; b then walks it.
// h uninstalls and reinstalls the app: last.
export const SCENARIOS = [a, g, b, c, d, e, f, h];
