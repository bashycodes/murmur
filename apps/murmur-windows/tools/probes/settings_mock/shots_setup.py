"""Screenshots of setup's speech-model step and its last step at the default
window size (1040 x 675), for before/after comparisons of a setup change.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/shots_setup.py [port] [prefix]
Writes shots/<prefix>-*.png (gitignored; prefix defaults to "setup"):
  3-fresh   a new install that picked This PC (step 3)
  3-rerun   Run setup on a PC whose speech runs on the server, with a
            whisper file of its own and this PC kept cold, switched to
            This PC (?whisper=turbo)
  5-rerun   Run setup with cleanup on your server, Rules only picked on the
            cleanup step, then on to the last step (?cleanup=server)
Selectors are the ones every setup version has (cards, options, Continue),
so the same script shoots before and after a change.
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
prefix = sys.argv[2] if len(sys.argv) > 2 else "setup"
out = HERE / "shots"
out.mkdir(exist_ok=True)


def step(pg):
    return pg.evaluate("[...document.querySelectorAll('#onboarding [data-ob-step]')].filter(p => !p.hidden).map(p => p.dataset.obStep).join(',')")


def nxt(pg, wait=300):
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
    pg.wait_for_timeout(wait)


def run_setup(pg, query):
    pg.goto(f"http://127.0.0.1:{port}/settings.html{query}")
    pg.wait_for_timeout(600)
    pg.evaluate("window.__goPage('settings-general')")
    pg.wait_for_timeout(200)
    pg.click("#run_setup")
    pg.wait_for_timeout(400)


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    pg = b.new_page(viewport={"width": 1040, "height": 675}, color_scheme="dark")
    pg.on("pageerror", lambda e: errors.append(str(e)))

    pg.goto(f"http://127.0.0.1:{port}/settings.html?onboard=1")
    pg.wait_for_timeout(600)
    nxt(pg)
    pg.click("[data-ob-engine=pc]")
    nxt(pg)
    pg.screenshot(path=str(out / f"{prefix}-3-fresh.png"))
    print("3-fresh:", step(pg))

    run_setup(pg, "?whisper=turbo")
    nxt(pg)
    pg.click("[data-ob-engine=pc]")
    nxt(pg)
    pg.wait_for_timeout(1500)
    pg.screenshot(path=str(out / f"{prefix}-3-rerun.png"))
    print("3-rerun:", step(pg))

    run_setup(pg, "?cleanup=server")
    nxt(pg)
    while step(pg) != "cleanup":
        nxt(pg)
    pg.click("[data-ob-cleanup=rules]")
    nxt(pg)
    pg.screenshot(path=str(out / f"{prefix}-5-rerun.png"))
    print("5-rerun:", step(pg))
    b.close()
    print("page errors:", errors or "none")
