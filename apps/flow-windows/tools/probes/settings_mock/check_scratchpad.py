"""Headless check of the Scratchpad page against mock-scratchpad.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_scratchpad.py [port] [screenshot.png]
Inserts text at the caret (what a dictation's Ctrl+V does), reloads the page
(window closed and opened again), checks the text is back, Copy all, Clear,
Undo clear and the word count, and prints what reached the mock shell.
Exits non-zero on the first failed check.
"""
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shot = sys.argv[2] if len(sys.argv) > 2 else None
url = f"http://127.0.0.1:{port}/scratchpad.html"


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        sys.exit(1)


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 520, "height": 560})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(url)
    pg.evaluate("localStorage.clear()")
    pg.reload()
    pg.wait_for_timeout(200)
    check(pg.input_value("#editor") == "", "empty on first open")
    check(pg.inner_text("#words") == "0 words", "0 words")
    check(pg.evaluate("document.activeElement.id") == "editor", "editor has focus")
    # A dictation pastes at the caret.
    pg.keyboard.insert_text("Hello world from the scratchpad.")
    pg.keyboard.press("Home")
    pg.keyboard.insert_text("Note: ")
    pg.wait_for_timeout(100)
    text = pg.input_value("#editor")
    check(text == "Note: Hello world from the scratchpad.", f"inserted at caret: {text!r}")
    check(pg.inner_text("#words") == "6 words", "word count 6")
    # Window closed (pagehide flushes) and opened again.
    pg.reload()
    pg.wait_for_timeout(200)
    check(pg.input_value("#editor") == text, "text restored after reopen")
    pg.click("#copy")
    pg.wait_for_timeout(100)
    check(pg.evaluate("window.__clipboard") == text, "Copy all = full text")
    check(pg.inner_text("#status") == "Copied", "status Copied")
    pg.click("#clear")
    pg.wait_for_timeout(100)
    check(pg.input_value("#editor") == "", "Clear empties")
    check(pg.is_visible("#undo"), "Undo clear shown")
    check(pg.inner_text("#words") == "0 words", "0 words after clear")
    if shot:
        pg.screenshot(path=shot)
    pg.click("#undo")
    pg.wait_for_timeout(100)
    check(pg.input_value("#editor") == text, "Undo restores")
    check(not pg.is_visible("#undo"), "Undo hidden again")
    pg.click("#clear")
    pg.keyboard.insert_text("new")
    pg.wait_for_timeout(100)
    check(not pg.is_visible("#undo"), "typing after Clear drops the undo")
    pg.reload()
    pg.wait_for_timeout(200)
    check(pg.input_value("#editor") == "new", "cleared + new text persisted")
    cmds = sorted({c[0] for c in pg.evaluate("window.__mockCalls")})
    print("commands used (last page):", cmds)
    check(errors == [], f"no page errors {errors}")
    b.close()
print("SCRATCHPAD CHECK PASSED")
