"""Headless check of "Keyboard language" (D37) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_keyboard_language.py [port]
Settings -> General -> Languages: Keyboard language is the first entry and
the default; the line says what the layout is now (English / Hebrew /
unknown) and re-reads it when the window gets the focus; a Hebrew layout
without a Hebrew model shows the download hint; picking another entry saves
keyboard_language off and back on. Writes shots/kbl-*.png (gitignored).
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def open_general(b, scheme, layout):
    pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
    pg.on("pageerror", lambda e: errors.append(str(e)))
    if layout != "default":
        pg.add_init_script(f"window.__keyboardLanguage = {layout};")
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__goPage('settings-general')")
    pg.wait_for_timeout(300)
    return pg


errors = []
with sync_playwright() as p:
    b = p.chromium.launch()
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = open_general(b, scheme, "default")
        opts = pg.eval_on_selector_all("#language_pick option", "os => os.map(o => [o.value, o.textContent])")
        check(tag + "Keyboard language is the first entry", opts[0] == ["keyboard", "Keyboard language"], str(opts))
        check(tag + "and the default", pg.input_value("#language_pick") == "keyboard" and prefs(pg)["keyboard_language"])
        desc = pg.inner_text("#languages_desc")
        check(tag + "says the layout now", "Keyboard language · now: English" in desc, desc)
        check(tag + "English layout: no Hebrew hint", not pg.is_visible("#languages_hebrew"))
        check(tag + "no codes field", not pg.is_visible("#languages_other"))
        if scheme == "light":
            pg.locator(".lw-section:has(#language_pick)").screenshot(path=str(out / "kbl-english.png"))
        # The user switches the layout and comes back to Settings.
        pg.evaluate("window.__keyboardLanguage = 'he'; window.dispatchEvent(new Event('focus'))")
        pg.wait_for_timeout(200)
        desc = pg.inner_text("#languages_desc")
        check(tag + "focus re-reads the layout", "now: Hebrew" in desc, desc)
        check(tag + "Hebrew layout, no Hebrew model: hint", pg.is_visible("#languages_hebrew"))
        if scheme == "light":
            pg.locator(".lw-section:has(#language_pick)").screenshot(path=str(out / "kbl-hebrew.png"))
        pg.select_option("#language_pick", "")
        pg.wait_for_timeout(200)
        p1 = prefs(pg)
        check(tag + "Detect automatically turns it off", not p1["keyboard_language"] and p1["languages"] == [], str(p1["keyboard_language"]))
        check(tag + "detect line", "Detected automatically" in pg.inner_text("#languages_desc"))
        pg.select_option("#language_pick", "en")
        pg.wait_for_timeout(200)
        check(tag + "English: fixed", prefs(pg)["languages"] == ["en"] and not prefs(pg)["keyboard_language"])
        pg.select_option("#language_pick", "keyboard")
        pg.wait_for_timeout(300)
        p2 = prefs(pg)
        check(tag + "back on: no fixed languages behind it", p2["keyboard_language"] and p2["languages"] == [], str(p2["languages"]))
        check(tag + "line back", "Keyboard language · now: Hebrew" in pg.inner_text("#languages_desc"), pg.inner_text("#languages_desc"))
        pg.close()
        pg = open_general(b, scheme, "null")
        desc = pg.inner_text("#languages_desc")
        check(tag + "unknown layout", "now: unknown" in desc and "detected automatically" in desc, desc)
        pg.close()
    b.close()
check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
