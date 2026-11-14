"""Headless check of the D96 naming: "Cloud" is OpenRouter only, the
OpenAI-compatible path is "Your own AI server", and the Transforms line and
Data and privacy follow the live engines.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_cloud_naming.py [port] [label]
Loads settings.html?server=1&cloud=custom (mock-tauri.js: own AI server at
10.0.0.5 picked with consent, cleanup order your server -> it -> this PC),
prints the labels in question and shoots each page to
shots/<label>-naming-<page>.png (gitignored). Then the same with OpenRouter
(no ?cloud), where "Cloud · OpenRouter" must stay.
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
label = sys.argv[2] if len(sys.argv) > 2 else "now"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []
check_mode = label != "before"


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def text(pg, sel):
    loc = pg.locator(sel)
    return loc.first.inner_text().strip() if loc.count() else ""


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1040, "height": 900})
    pg.goto(f"http://127.0.0.1:{port}/settings.html?server=1&cloud=custom")
    pg.wait_for_timeout(700)
    seen = {}
    for page in ["settings-openrouter", "settings-cleanup", "settings-speech", "settings-privacy", "transforms"]:
        pg.evaluate(f"window.__goPage('{page}')")
        pg.wait_for_timeout(400)
        pg.screenshot(path=str(out / f"{label}-naming-{page}.png"), full_page=True)
    nav = pg.locator('[data-page="settings-openrouter"]').first.inner_text().strip()
    seen["nav tab"] = nav
    seen["provider button"] = text(pg, '#cloud_provider_seg [data-cloud-provider="custom"]')
    seen["cleanup row title"] = text(pg, "#aieng_row_cloud .fl-row-title")
    seen["cleanup row desc"] = text(pg, "#aieng_desc_cloud")
    seen["cleanup card title"] = text(pg, "#or_cleanup_title")
    seen["speech row title"] = text(pg, "#eng_row_cloud .fl-row-title")
    seen["engines card ai"] = text(pg, "#eng_ai")
    seen["transforms line"] = text(pg, "#tf_engines")
    for k in ["leave_server_cleanup", "leave_or_text", "leave_or_audio"]:
        loc = pg.locator(f"#{k}")
        shown = loc.count() and not loc.evaluate("e => e.hidden")
        seen[k] = " | ".join(loc.evaluate("e => [...e.querySelectorAll('.fl-row-title, .fl-row-desc, .fl-badge')].map(n => n.textContent.trim())")) if shown else "(hidden/absent)"
    for k, v in seen.items():
        print(f"  {k}: {v}")
    if check_mode:
        check("no 'Cloud' on the own-AI-server cleanup row", "Cloud" not in seen["cleanup row title"], seen["cleanup row title"])
        check("speech row names own AI server", seen["speech row title"] == "Your own AI server", seen["speech row title"])
        check("own AI server named", "Your own AI server" in seen["provider button"], seen["provider button"])
        check("cleanup card title names own AI server", "Your own AI server" in seen["cleanup card title"] and "Cloud" not in seen["cleanup card title"], seen["cleanup card title"])
        check("engines card: your server first", seen["engines card ai"].startswith("Your server"), seen["engines card ai"])
        check("transforms line from the order", "your own AI server, then this PC" in seen["transforms line"] and "cloud" not in seen["transforms line"].lower(), seen["transforms line"])
        check("Text to your server row shown", seen["leave_server_cleanup"].startswith("Text to your server"), seen["leave_server_cleanup"])
        check("text row names own AI server", "own AI server" in seen["leave_or_text"], seen["leave_or_text"])

    pg.goto(f"http://127.0.0.1:{port}/settings.html?server=1")
    pg.wait_for_timeout(700)
    pg.evaluate("window.__goPage('settings-cleanup')")
    pg.wait_for_timeout(300)
    or_title = text(pg, "#aieng_row_cloud .fl-row-title")
    print(f"  openrouter cleanup row title: {or_title}")
    pg.evaluate("window.__goPage('transforms')")
    pg.wait_for_timeout(300)
    tf = text(pg, "#tf_engines")
    print(f"  openrouter transforms line: {tf}")
    if check_mode:
        check("OpenRouter keeps 'Cloud'", or_title.startswith("Cloud"), or_title)

    # Setup: the speech card (step 2) and the cleanup option (step 4).
    flat = "e => e.textContent.replace(/\\s+/g, ' ').trim()"
    for q, own in [("onboard=1&cloud=custom", True), ("onboard=1", False)]:
        pg.goto(f"http://127.0.0.1:{port}/settings.html?{q}")
        pg.wait_for_timeout(700)
        pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
        pg.wait_for_timeout(300)
        card = pg.eval_on_selector("[data-ob-engine=cloud]", flat)
        pg.evaluate("window.__obCleanupText = () => document.querySelector('[data-ob-cleanup=cloud] [data-ob-title]').textContent + ' | ' + document.querySelector('[data-ob-cleanup=cloud] [data-ob-desc]').textContent")
        # Fastest way to step 4: your server (address + Test); D94 skips the model step.
        pg.click("[data-ob-engine=server]")
        pg.fill("#ob_host", "100.64.0.10")
        pg.fill("#ob_token", "abcdefghijklmnop1234")
        pg.click("#ob_test_conn")
        pg.wait_for_timeout(200)
        pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
        pg.wait_for_timeout(300)
        opt = pg.evaluate("window.__obCleanupText()")
        print(f"  setup ({'own' if own else 'openrouter'}): card: {card}  ||  cleanup: {opt}")
        pg.screenshot(path=str(out / f"{label}-naming-setup-{'own' if own else 'or'}.png"), full_page=True)
        if check_mode and own:
            check("setup card not 'Cloud'/'online' for own server", "Your own AI server" in card and "online" not in card and "Cloud" not in card, card)
            check("setup cleanup option not 'Cloud'/'online'", opt.startswith("Your own AI server") and "online" not in opt, opt)
        if check_mode and not own:
            check("setup keeps 'Cloud' for OpenRouter", "Cloud" in card and opt.startswith("Cloud"), f"{card} / {opt}")
    b.close()

if fails:
    print(f"{len(fails)} failed")
    sys.exit(1)
