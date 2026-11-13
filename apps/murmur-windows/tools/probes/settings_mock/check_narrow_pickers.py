"""Headless check that Speech to text and AI cleanup fit a narrow window.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_narrow_pickers.py [port]
At 1040px wide, with the Advanced folds closed and open, no select is cut off
(its text wider than the box, or past the window edge) and the page has no
horizontal scroll. Writes shots/narrow-*.png (gitignored).
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = pathlib.Path(__file__).parent / "shots"
out.mkdir(exist_ok=True)
fails = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f"  ({detail})" if detail and not ok else ""))
    if not ok:
        fails.append(name)


CUT = """[...document.querySelectorAll('select')]
  .filter(s => s.offsetParent && (s.scrollWidth > s.clientWidth + 1
    || s.getBoundingClientRect().right > document.documentElement.clientWidth))
  .map(s => s.id)"""

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1040, "height": 900})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html?server=1")
    pg.wait_for_timeout(600)
    for page in ["settings-speech", "settings-cleanup"]:
        for state in ["closed", "open"]:
            pg.evaluate(f"window.__goPage('{page}'); document.querySelectorAll('details.lw-fold').forEach(d => d.open = {'true' if state == 'open' else 'false'})")
            pg.wait_for_timeout(300)
            pg.screenshot(path=str(out / f"narrow-{page}-{state}.png"), full_page=True)
            cut = pg.evaluate(CUT)
            check(f"{page} ({state}): no select cut off", not cut, ", ".join(cut))
            check(f"{page} ({state}): no horizontal scroll", not pg.evaluate("document.documentElement.scrollWidth > document.documentElement.clientWidth"))
    check("no page errors", not errors, "; ".join(errors[:3]))
    b.close()

print("all ok" if not fails else "FAILED: " + ", ".join(fails))
sys.exit(1 if fails else 0)
