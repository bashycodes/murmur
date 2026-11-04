"""Needs the Notetaker switched on (D51: `NOTETAKER_ENABLED = true` in
src/meetings/mod.rs and ui/src/pages/notetaker.ts, then rebuild ui/dist).

Headless check of the Meetings page (Notetaker, D45) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_meetings.py [port]
Checks the list (date, duration, app, failed status), opening a transcript
with Me / Others turns, copy, delete (after the confirm dialog), start and
stop from the page with the live indicator, the meeting detection setting
(D48: off by default, saved to prefs), and the meeting shortcut row in
Settings -> General. Exits non-zero on the first failed check. Optional
second argument: a path for a screenshot of the open transcript.
"""
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shot = sys.argv[2] if len(sys.argv) > 2 else None


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        sys.exit(1)


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(300)
    nav = pg.locator('.lw-sidebar [data-page="meetings"]')
    check(nav.inner_text().strip() == "Meetings", f"nav says Meetings: {nav.inner_text()!r}")
    nav.click()
    pg.wait_for_timeout(200)
    check(pg.is_visible("#page_meetings"), "Meetings page shown")
    rows = pg.locator("#mt_list .lw-mt-row")
    check(rows.count() == 2, f"two meetings listed ({rows.count()})")
    first = rows.nth(0).inner_text()
    check("25 min" in first and "teams" in first, f"duration + app: {first!r}")
    check("Thanks for joining" in first, "preview line")
    second = rows.nth(1).inner_text()
    check("Failed: speech model not found" in second, f"failed status: {second!r}")
    check(pg.locator('#mt_list [data-id="m1"] [aria-label="Transcribe again"]').count() == 1, "retry on a failed meeting")

    # Open the transcript.
    pg.click('#mt_list [data-id="m2"] .lw-mt-open')
    pg.wait_for_timeout(200)
    check(pg.is_visible("#mt_detail") and not pg.is_visible("#mt_list_view"), "transcript view")
    check(pg.inner_text("#mt_sub").endswith("Transcribed on: This PC"), "route that transcribed it")
    who = pg.eval_on_selector_all("#mt_turns .lw-mt-who", "els => els.map(e => e.textContent)")
    check(who == ["Me", "Others", "Me"], f"speakers in order: {who}")
    stamps = pg.eval_on_selector_all("#mt_turns .lw-mt-stamp", "els => els.map(e => e.textContent)")
    check(stamps == ["0:01", "0:05", "0:12"], f"stamps: {stamps}")
    if shot:
        pg.screenshot(path=shot, full_page=True)
    pg.click("#mt_copy")
    pg.wait_for_timeout(100)
    copied = pg.evaluate("window.__copied") or ""
    check(copied.startswith("Me: Thanks") and "\nOthers: Sounds good" in copied, "copy")

    # Delete: cancel keeps it, OK removes it.
    pg.click("#mt_delete")
    pg.wait_for_timeout(100)
    check(pg.evaluate("document.getElementById('mt_del_dlg').open"), "confirm dialog")
    pg.click("#mt_del_cancel")
    pg.wait_for_timeout(100)
    check(pg.is_visible("#mt_detail"), "cancel keeps the meeting")
    pg.click("#mt_delete")
    pg.wait_for_timeout(100)
    pg.click("#mt_del_ok")
    pg.wait_for_timeout(200)
    check(pg.is_visible("#mt_list_view") and rows.count() == 1, "deleted, back on the list")

    # Start / stop from the page.
    pg.click("#mt_toggle")
    pg.wait_for_timeout(200)
    check(pg.is_visible("#mt_live") and "Recording" in pg.inner_text("#mt_live"), "recording indicator")
    check(pg.inner_text("#mt_toggle") == "Stop recording", "button says Stop")
    pg.click("#mt_toggle")
    pg.wait_for_timeout(200)
    check(pg.inner_text("#mt_toggle") == "Start recording", "stopped")
    check(rows.count() == 2 and "Waiting to be transcribed" in rows.nth(0).inner_text(), "new meeting queued")

    # Meeting detection (D48): off by default, saved to prefs.
    check(pg.input_value("#mt_detect") == "off", "detection off by default")
    pg.select_option("#mt_detect", "ask")
    pg.wait_for_timeout(200)
    saved = pg.evaluate("window.__TAURI__.core.invoke('get_prefs').then(p => p.meeting_detection)")
    check(saved == "ask", f"detection saved: {saved!r}")

    # The shortcut row.
    pg.evaluate("window.__goPage('settings-general')")
    pg.wait_for_timeout(200)
    check(pg.locator("#meeting").count() == 1, "meeting shortcut field in General")
    check(errors == [], f"no page errors {errors}")
    b.close()
print("MEETINGS PAGE CHECK PASSED")
