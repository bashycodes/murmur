"""Screenshot every page of the app window against mock-tauri.js, light and dark.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/shots.py [port] [page ...]
Writes shots/<page>-<light|dark>.png next to this file (gitignored) at the
window's default size, 1280x800, and prints any page errors.
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
pages = sys.argv[2:] or ["home", "settings-general"]
out = HERE / "shots"
out.mkdir(exist_ok=True)

with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ("light", "dark"):
        pg = b.new_page(viewport={"width": 1280, "height": 800}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.on("console", lambda m: m.type == "error" and errors.append(m.text))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(400)
        for page in pages:
            pg.evaluate("id => window.__goPage(id)", page)
            pg.wait_for_timeout(300)
            path = out / f"{page}-{scheme}.png"
            pg.screenshot(path=str(path))
            print(path.relative_to(HERE))
        pg.close()
    print("page errors:", errors)
    b.close()
