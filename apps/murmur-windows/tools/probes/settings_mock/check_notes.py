"""Headless check of the Notes list (scratchpad.html?view=notes) against
mock-scratchpad.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_notes.py [port] [screenshot.png]
Keeps a Quick note, creates three notes through New note (typing into each
editor and going back), reloads (restart), then checks the list order, search
by content, Pin, Export and the two-click Delete, and that the Quick note has
no Delete. Exits non-zero on the first failed check.
"""
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shot = sys.argv[2] if len(sys.argv) > 2 else None
base = f"http://127.0.0.1:{port}/scratchpad.html"


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        sys.exit(1)


def titles(pg):
    return pg.eval_on_selector_all("#notes .title", "els => els.map(e => e.firstChild.textContent)")


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 520, "height": 560})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(base)
    pg.evaluate("localStorage.clear()")
    # The Quick note (what tray "Scratchpad…" opens), as before Notes.
    pg.reload()
    pg.wait_for_timeout(150)
    check(pg.inner_text("#note-name") == "Quick note", "default page = Quick note editor")
    pg.keyboard.insert_text("Quick thought")
    pg.click("#back")
    pg.wait_for_url("**view=notes")
    pg.wait_for_timeout(150)
    check(titles(pg) == ["Quick note"], f"list has the Quick note: {titles(pg)}")
    for text in ["Groceries\nmilk eggs bread", "Meeting with Dana\nTuesday 10:00", "Idea: dictate tests"]:
        pg.click("#new-note")
        pg.wait_for_url("**note=note-*")
        pg.wait_for_timeout(150)
        check(pg.evaluate("document.activeElement.id") == "editor", "new note editor focused")
        pg.keyboard.insert_text(text)
        pg.wait_for_timeout(50)
        pg.click("#back")
        pg.wait_for_url("**view=notes")
        pg.wait_for_timeout(150)
    # Restart: the page loads again from storage.
    pg.reload()
    pg.wait_for_timeout(200)
    t = titles(pg)
    check(t == ["Idea: dictate tests", "Meeting with Dana", "Groceries", "Quick note"], f"4 notes newest first: {t}")
    check("4 words" in pg.inner_text("#notes li:nth-child(3) .meta"), "word count in the row")
    check("4 notes" == pg.inner_text("#count"), "count")
    check(pg.locator("#notes li:last-child .delete").count() == 0, "Quick note has no Delete")
    # Search by content (a body line, not just the title).
    pg.fill("#search", "tuesday")
    pg.wait_for_timeout(300)
    check(titles(pg) == ["Meeting with Dana"], f"search tuesday: {titles(pg)}")
    pg.fill("#search", "QUICK")
    pg.wait_for_timeout(300)
    check(titles(pg) == ["Quick note"], "search is case-insensitive")
    pg.fill("#search", "zzz")
    pg.wait_for_timeout(300)
    check(pg.is_visible("#empty"), "no match message")
    pg.fill("#search", "")
    pg.wait_for_timeout(300)
    # Pin the oldest note: it moves to the top.
    pg.click("#notes li:nth-child(3) .pin")
    pg.wait_for_timeout(150)
    check(titles(pg)[0] == "Groceries", f"pinned first: {titles(pg)}")
    check(pg.inner_text("#notes li:first-child .pin") == "Unpin", "Unpin shown")
    # Export.
    pg.select_option("#notes li:first-child select.export", "md")
    pg.wait_for_timeout(150)
    ex = pg.evaluate("window.__exports")
    check(ex and ex[-1][0].endswith("Groceries.md") and ex[-1][1].startswith("Groceries\nmilk"), f"export .md {ex}")
    check("Saved" in pg.inner_text("#list-status"), "export status")
    # Delete needs a second click.
    pg.click("#notes li:nth-child(2) .delete")
    pg.wait_for_timeout(100)
    check(len(titles(pg)) == 4, "first click only arms Delete")
    check(pg.inner_text("#notes li:nth-child(2) .delete") == "Delete?", "armed label")
    if shot:
        pg.screenshot(path=shot)
    pg.click("#notes li:nth-child(2) .delete")
    pg.wait_for_timeout(150)
    t = titles(pg)
    check(t == ["Groceries", "Meeting with Dana", "Quick note"], f"deleted: {t}")
    left = pg.evaluate("Object.keys(localStorage).filter(k => k.startsWith('mock-note:')).length")
    check(left == 3, f"the note's file is gone ({left} stored)")
    # Open a note from the list: its editor, with its text.
    pg.click("#notes li:nth-child(2) .open")
    pg.wait_for_url("**note=note-*")
    pg.wait_for_timeout(150)
    check(pg.input_value("#editor") == "Meeting with Dana\nTuesday 10:00", "opens the note's text")
    check(pg.inner_text("#note-name") == "Meeting with Dana", "editor shows the title")
    pg.click("#export-txt")
    pg.wait_for_timeout(100)
    check(pg.evaluate("window.__exports")[-1][0].endswith("Meeting with Dana.txt"), "editor export .txt")
    check(errors == [], f"no page errors {errors}")
    b.close()
print("NOTES CHECK PASSED")
