"""Screenshots of Settings pages in a small window, the whole scrolled panel.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/shoot_settings_pages.py [port] [label] [page ...]
At 1040x675 (the app's small window), with a server set, each page's
settings panel (#settings_body) is scrolled from top to bottom and the
window is shot at every step: shots/<label>-<page>-<n>.png (gitignored).
Folds stay as the page draws them (closed). Default pages: Speech to text
and AI cleanup; default label: "now". Compare a branch with another by
serving that tree's ui/ and passing another label.
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
label = sys.argv[2] if len(sys.argv) > 2 else "now"
pages = sys.argv[3:] or ["settings-speech", "settings-cleanup"]
out = pathlib.Path(__file__).parent / "shots"
out.mkdir(exist_ok=True)

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1040, "height": 675})
    pg.goto(f"http://127.0.0.1:{port}/settings.html?server=1")
    pg.wait_for_timeout(600)
    for page in pages:
        pg.evaluate(f"window.__goPage('{page}')")
        pg.wait_for_timeout(400)
        body = pg.locator("#settings_body")
        height, view = body.evaluate("b => [b.scrollHeight, b.clientHeight]")
        step = max(view - 80, 100)
        n, top = 0, 0
        while True:
            body.evaluate(f"b => b.scrollTop = {top}")
            pg.wait_for_timeout(150)
            n += 1
            path = out / f"{label}-{page}-{n}.png"
            pg.screenshot(path=str(path))
            print(path)
            if top + view >= height:
                break
            top += step
    b.close()
