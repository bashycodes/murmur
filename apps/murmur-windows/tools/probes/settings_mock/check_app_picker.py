"""Headless check of the "Style per app" picker against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_app_picker.py [port] [screenshot.png]
Loads the mock prefs as rows, adds a recent app, rejects a duplicate and a
bad manual entry, removes a row, overrides a built-in one (each autosaves), and
prints the app_buckets that reached set_prefs.
"""
import json
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shot = sys.argv[2] if len(sys.argv) > 2 else None

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1280, "height": 1400})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.click("[data-page=style]")
    pg.wait_for_timeout(300)
    rows = lambda: pg.eval_on_selector_all(
        "#app_rows tr",
        "trs => trs.map(t => [t.cells[0].innerText, t.querySelector('select').value])",
    )
    print("loaded", rows())
    print("recent", pg.eval_on_selector_all("#app_recent option", "o => o.map(x => x.text)"))
    pg.select_option("#app_recent", "notepad")
    pg.click("#app_add_recent")
    pg.fill("#app_manual", "MyChat.exe")
    pg.press("#app_manual", "Enter")
    print("dup status:", pg.inner_text("#app_add_status"))
    pg.fill("#app_manual", "a=b")
    pg.click("#app_add_manual")
    print("bad status:", pg.inner_text("#app_add_status"))
    pg.fill("#app_manual", r"https://www.Crm.Example.com/inbox")
    pg.click("#app_add_manual")
    # Remove mail.example.com, override the built-in Telegram row.
    pg.click("#app_rows tr:has-text('mail.example.com') button")
    pg.click("details.defaults summary")
    pg.click("#app_default_rows tr:has-text('Telegram') button")
    pg.select_option("#app_rows tr:has-text('Telegram') select", "work")
    print("rows", rows())
    print("defaults overridden:", pg.eval_on_selector_all(
        "#app_default_rows tr.overridden", "t => t.map(x => x.cells[0].innerText)"))
    pg.wait_for_timeout(200)  # Style autosaves on every change.
    print("status:", pg.inner_text("#style_status"))
    saved = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")
    print("saved app_buckets", json.dumps(saved["app_buckets"], sort_keys=True))
    if shot:
        pg.locator(".lw-section:has(#app_rows)").screenshot(path=shot)
    print("page errors:", errors)
    b.close()
