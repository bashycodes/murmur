"""Headless check of the app window's Scratchpad page against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_scratchpad_page.py [port]
Checks the pinned / recent lists, the configured shortcut, opening a note
(`note_open`), New note (`note_create` then `note_open`), pin, search,
and that the page never asks for a note's text (`note_load` etc. are the
notes window's only). With the Notetaker hidden (D51, `NOTETAKER_ENABLED`
false) also checks its sidebar entry opens the static preview page, that
no Meetings page exists and no meeting command is called, and that the
meeting shortcut row and the Speech to text Meetings group are hidden.
Exits non-zero on the first failed check.
"""
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        sys.exit(1)


def titles(pg, list_id):
    return pg.eval_on_selector_all(f"#{list_id} .lw-sp-title", "els => els.map(e => e.textContent)")


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(300)
    pg.click('.lw-sidebar [data-page="scratchpad"]')
    pg.wait_for_timeout(200)
    check(pg.is_visible("#page_scratchpad"), "Scratchpad page shown")
    check(titles(pg, "sp_pinned") == ["Release checklist"], f"pinned: {titles(pg, 'sp_pinned')}")
    check(titles(pg, "sp_recent") == ["Quick note", "Reply to Sam", "Ideas"], f"recent: {titles(pg, 'sp_recent')}")
    check(pg.inner_text("#sp_keys").split() == ["Ctrl", "Alt", "N"], "shortcut keys shown")
    check(pg.inner_text("#sp_shortcut") == "Change", "shortcut button says Change")
    # Open a note.
    pg.click('#sp_recent [data-id="note-2"] .lw-sp-open')
    pg.wait_for_timeout(100)
    check(pg.evaluate("window.__opened") == ["note-2"], "row opens its note")
    # New note.
    pg.click("#sp_new")
    pg.wait_for_timeout(200)
    opened = pg.evaluate("window.__opened")
    check(len(opened) == 2 and opened[1].startswith("note-"), f"new note opened: {opened}")
    check("Untitled" in titles(pg, "sp_recent"), "new note listed")
    # Pin, then unpin.
    pg.hover('#sp_recent [data-id="note-3"]')
    pg.click('#sp_recent [data-id="note-3"] .lw-sp-pin')
    pg.wait_for_timeout(150)
    check("Ideas" in titles(pg, "sp_pinned"), "pinned moves to Pinned")
    pg.click('#sp_pinned [data-id="note-3"] .lw-sp-pin')
    pg.wait_for_timeout(150)
    check("Ideas" in titles(pg, "sp_recent"), "unpinned back in Recent")
    # Search.
    pg.click("#sp_search_btn")
    check(pg.is_visible("#sp_search"), "search opens")
    pg.fill("#sp_search", "slides")
    pg.wait_for_timeout(300)
    check(titles(pg, "sp_recent") == ["Reply to Sam"] and not pg.is_visible("#sp_pinned_sec"), "search filters")
    pg.fill("#sp_search", "zzz")
    pg.wait_for_timeout(300)
    check(pg.is_visible("#sp_empty"), "no match message")
    pg.press("#sp_search", "Escape")
    pg.wait_for_timeout(300)
    check(not pg.is_visible("#sp_search") and len(titles(pg, "sp_recent")) >= 4, "Esc closes search, list back")
    check(pg.is_visible("#page_scratchpad"), "Esc did not leave the page")
    cmds = sorted({c[0] for c in pg.evaluate("window.__mockCalls") if c[0].startswith("note_")})
    check(cmds == ["note_create", "note_list", "note_open", "note_set_pinned"], f"note commands used: {cmds}")
    # D51: the Notetaker is hidden behind its preview page.
    check(pg.query_selector('.lw-sidebar [data-page="meetings"]') is None, "no Meetings entry")
    check(pg.query_selector("#page_meetings") is None, "no Meetings page")
    pg.click('.lw-sidebar [data-page="notetaker"]')
    pg.wait_for_timeout(100)
    check(pg.is_visible("#page_notetaker"), "Notetaker preview page shown")
    check(pg.is_visible("#page_notetaker .fl-badge"), "preview has its Coming later badge")
    meet = sorted({c[0] for c in pg.evaluate("window.__mockCalls") if c[0].startswith("meeting_")})
    check(meet == [], f"no meeting commands called: {meet}")
    check(pg.eval_on_selector('[data-edit="meeting"]', "e => e.closest('.fl-row').hidden"), "meeting shortcut row hidden")
    check(pg.eval_on_selector("#meet_group", "e => e.hidden"), "Speech to text Meetings group hidden")
    check(errors == [], f"no page errors {errors}")
    b.close()
print("SCRATCHPAD PAGE CHECK PASSED")
