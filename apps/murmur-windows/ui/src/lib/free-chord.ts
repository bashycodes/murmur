// The shortcut a new transform is offered (D109): the first Win+Shift
// letter that neither Windows nor another Murmur shortcut uses. Pure, so
// tests/free-chord.test.mjs can run it.

/** Win+Shift letters Windows takes: A (tips), C (Windows 10 charms),
 *  M (restore windows), R (record a region), S (snip), T (taskbar),
 *  V (notifications). Win+Shift+digit (new app instance) is not offered
 *  either: only letters are. */
const WINDOWS_TAKES = new Set(["A", "C", "M", "R", "S", "T", "V"]);

/** "Win+Shift+B", or "" (no default) when every free letter is used.
 *  `used` holds every Murmur shortcut (any case, `null` / "" = none). */
export function firstFreeChord(used: Iterable<string | null | undefined>): string {
  const taken = new Set([...used].filter((c): c is string => !!c).map((c) => c.trim().toLowerCase()));
  for (let code = 65; code <= 90; code++) {
    const letter = String.fromCharCode(code);
    const chord = `Win+Shift+${letter}`;
    if (!WINDOWS_TAKES.has(letter) && !taken.has(chord.toLowerCase())) return chord;
  }
  return "";
}
