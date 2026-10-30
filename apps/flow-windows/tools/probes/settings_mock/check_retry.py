"""Headless check of History "Retry" on Home against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_retry.py [port]
Home offers Retry only on rows whose recording is kept (not on commands
or rows without audio); a retried row shows numbered version pills
(0 = the original) whose clicks swap the text in place (view only);
a retry adds the next pill and shows the newest text; a failed retry says
why; Settings -> Data and privacy saves "Keep recordings for retry".
Then writes shots/retry-*.png (gitignored) in light and dark.
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
    print(("ok   " if cond else "FAIL ") + name + (f"  {extra}" if extra and not cond else ""))
    if not cond:
        fails.append(name)


ROW = '#hist_list .lw-hist-row[data-id="{}"]'

with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.on("console", lambda m: m.type == "error" and errors.append(m.text))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(400)
    pg.click("[data-page=home]")
    pg.wait_for_timeout(300)

    with_retry = pg.eval_on_selector_all(
        "#hist_list .lw-hist-row", "rs => rs.filter(r => r.querySelector('.lw-retry')).map(r => +r.dataset.id)"
    )
    check("Retry only where the recording is kept", sorted(with_retry) == [3, 5], with_retry)
    label = pg.get_attribute(ROW.format(5) + " .lw-retry", "aria-label")
    check("Retry is labelled", label == "Retry with current settings", label)
    order = pg.eval_on_selector_all(ROW.format(5) + " .lw-hist-actions button", "bs => bs.map(b => b.title)")
    check("Retry sits beside Copy / Paste again", order[:3] == ["Copy", "Paste again", "Retry with current settings"], order)

    tabs = lambda id: pg.eval_on_selector_all(ROW.format(id) + " .lw-ver-tab", "ts => ts.map(t => t.textContent)")
    selected = lambda id: pg.eval_on_selector_all(
        ROW.format(id) + " .lw-ver-tab", "ts => ts.findIndex(t => t.getAttribute('aria-selected') === 'true')"
    )
    text = lambda id: pg.inner_text(ROW.format(id) + " .lw-hist-text")
    check("retried row: pills 0 1", tabs(5) == ["0", "1"], tabs(5))
    check("never retried: no pills", tabs(3) == [], tabs(3))
    check("newest selected", selected(5) == 1)
    newest = text(5)
    pg.click(ROW.format(5) + " .lw-ver-tab >> nth=0")
    check("pill 0 shows the original", "bench marks" in text(5), text(5))
    check("old version is dimmed", "is-old" in pg.get_attribute(ROW.format(5) + " .lw-hist-text", "class"))
    check("no restore button", pg.query_selector(ROW.format(5) + " .lw-ver button:not(.lw-ver-tab)") is None)
    pg.screenshot(path=str(out / "retry-version0-light.png"))
    pg.click(ROW.format(5) + " .lw-ver-tab >> nth=1")
    check("pill 1 back to the newest", text(5) == newest)

    pg.hover(ROW.format(3))
    pg.click(ROW.format(3) + " .lw-retry")
    pg.wait_for_timeout(50)
    check("busy while retrying", pg.is_disabled(ROW.format(3) + " .lw-retry"))
    check("says it is retrying", "Retrying" in pg.inner_text("#hist_status"))
    pg.wait_for_timeout(600)
    check("retry adds pills 0 1", tabs(3) == ["0", "1"], tabs(3))
    check("newest text shown", "(retry 1)" in text(3), text(3))
    check("newest selected after retry", selected(3) == 1)
    check("done message", "newest version" in pg.inner_text("#hist_status"), pg.inner_text("#hist_status"))
    check("never pastes", pg.evaluate("window.__retries") == 1)
    pg.hover(ROW.format(3))
    pg.click(ROW.format(3) + " .lw-retry")
    pg.wait_for_timeout(600)
    check("a failed retry says why", "No speech found" in pg.inner_text("#hist_status"), pg.inner_text("#hist_status"))
    check("failed retry: button usable again", pg.is_enabled(ROW.format(3) + " .lw-retry"))
    check("failed retry: versions unchanged", tabs(3) == ["0", "1"], tabs(3))

    pg.evaluate("window.__goPage('settings-privacy')")
    pg.wait_for_timeout(200)
    check("keep recordings: default 20", pg.input_value("#retry_audio_keep") == "20")
    pg.select_option("#retry_audio_keep", "5")
    pg.wait_for_timeout(150)
    saved = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")["retry_audio_keep"]
    check("keep recordings saved", saved == 5, saved)
    check("no 'audio is never stored' claim", "audio is never stored" not in pg.inner_text("body"))
    pg.close()

    for scheme in ("light", "dark"):
        pg = b.new_page(viewport={"width": 1280, "height": 800}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(400)
        pg.click("[data-page=home]")
        pg.wait_for_timeout(300)
        pg.hover(ROW.format(5))
        pg.screenshot(path=str(out / f"retry-{scheme}.png"))
        pg.close()
    print("shots in", out.relative_to(HERE.parents[2]))
    print("page errors:", errors)
    b.close()
    if fails or errors:
        sys.exit(1)
