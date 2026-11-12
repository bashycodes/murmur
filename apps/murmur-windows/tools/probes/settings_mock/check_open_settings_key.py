"""Headless check of Ctrl+, (open Settings) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_open_settings_key.py [port] [screenshot.png]
From the main window Ctrl+, opens Settings on the last settings page, again
does nothing, Esc closes it, Ctrl+Shift+, does nothing, Settings → General
lists the key, and it does nothing under first-run setup (?onboard=1).
Exits non-zero on a failed check.
"""
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shot = sys.argv[2] if len(sys.argv) > 2 else None
failed = []


def check(name, ok):
    print(("ok  " if ok else "FAIL"), name)
    if not ok:
        failed.append(name)


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1280, "height": 900})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(300)
    open_ = lambda: pg.evaluate("!document.getElementById('settings').hidden")
    pg.click("[data-page=dictionary]")
    check("starts closed", not open_())
    pg.keyboard.press("Shift+Control+Comma")
    check("Ctrl+Shift+, does nothing", not open_())
    pg.keyboard.press("Control+Comma")
    check("Ctrl+, opens Settings", open_())
    check("on General", pg.is_visible("#page_settings-general"))
    pg.click("#settings_nav [data-page=settings-system]")
    pg.keyboard.press("Control+Comma")
    check("Ctrl+, again stays on the page", open_() and pg.is_visible("#page_settings-system"))
    pg.keyboard.press("Escape")
    check("Esc closes", not open_() and pg.is_visible("#page_dictionary"))
    pg.keyboard.press("Control+Comma")
    check("reopens on the last settings page", pg.is_visible("#page_settings-system"))
    pg.click("#settings_nav [data-page=settings-general]")
    keys = pg.inner_text("#open_settings_keys")
    check(f"General lists the key ({keys!r})", "Ctrl" in keys and "," in keys)
    if shot:
        pg.locator(".lw-section:has(#open_settings_keys)").screenshot(path=shot)
    # First-run setup over the shell: Ctrl+, must not open Settings under it.
    pg.goto(f"http://127.0.0.1:{port}/settings.html?onboard=1")
    pg.wait_for_timeout(300)
    pg.keyboard.press("Control+Comma")
    check("not under first-run setup", pg.is_visible("#onboarding") and not open_())
    check(f"no page errors {errors}", not errors)
    b.close()

sys.exit(1 if failed else 0)
