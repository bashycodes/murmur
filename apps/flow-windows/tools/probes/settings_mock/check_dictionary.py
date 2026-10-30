"""Headless check of the Dictionary and Snippets pages against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_dictionary.py [port]
Adds a word, a "heard as" replacement and a snippet through the add card,
edits and deletes rows (each saves at once, Undo restores), filters by tab
and search, sorts A-Z, toggles "Learn from my corrections", accepts a
suggestion and imports a file (merge, then replace all). Asserts what
reached set_prefs. Then fills the lists with the artboards' sample data and
writes shots/dict-*.png (gitignored) in light and dark: the lists, the add
cards open, and the empty states.
"""
import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)

# The artboards' sample rows (PageDictionary.dc.html, PageSnippets.dc.html).
SAMPLE = [
    {"word": "Claude", "replacement": None, "isSnippet": False},
    {"word": "Parakeet", "replacement": None, "isSnippet": False},
    {"word": "jason", "replacement": "JSON", "isSnippet": False},
    {"word": "Hetzner", "replacement": None, "isSnippet": False},
    {"word": "Alex Rivera", "replacement": None, "isSnippet": False},
    {"word": "tail scale", "replacement": "Tailscale", "isSnippet": False},
    {"word": "n8n", "replacement": None, "isSnippet": False},
    {"word": "TickTick", "replacement": None, "isSnippet": False},
    {"word": "my email address", "replacement": "me@example.com", "isSnippet": True},
    {"word": "my sign-off", "replacement": "Thanks, Alex", "isSnippet": True},
    {"word": "bug report template", "replacement": "Steps to reproduce: … Expected: … Actual: … Version: …", "isSnippet": True},
    {"word": "my home address", "replacement": "[YOUR ADDRESS]", "isSnippet": True},
]

fails = []


def replace_with(pg, entries):
    pg.set_input_files("#dict_file_replace", files=[{"name": "d.json", "mimeType": "application/json", "buffer": json.dumps(entries).encode()}])
    pg.wait_for_timeout(250)
    pg.evaluate("document.getElementById('dict_status').textContent = ''; document.getElementById('io_status').textContent = ''")


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  {extra}" if extra and not cond else ""))
    if not cond:
        fails.append(name)


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.on("console", lambda m: m.type == "error" and errors.append(m.text))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(400)
    saved = lambda: pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")
    texts = lambda sel: pg.eval_on_selector_all(sel, "rs => rs.map(r => r.innerText.replace(/\\s+/g, ' ').trim())")

    pg.evaluate("window.__goPage('dictionary')")
    pg.wait_for_timeout(200)
    check("rows render heard → word", texts("#dict_rows .lw-dict-row") == ["Kubernetes", "gh → GitHub"], texts("#dict_rows .lw-dict-row"))
    check("suggestion shown", texts("#sugg_list li .lw-dict-text") == ["whisper flow → Wispr Flow"])

    # Add a plain word, then a replacement.
    pg.click("#dict_add")
    check("add card opens, focused", pg.evaluate("document.activeElement.closest('#dict_form_slot') !== null"))
    pg.click("#dict_form_slot button[type=submit]")
    check("empty word rejected", "Fill in the word" in pg.inner_text("#dict_form_slot"))
    pg.locator("#dict_form_slot input").first.fill("Hetzner")
    pg.locator("#dict_form_slot input").first.press("Enter")
    pg.wait_for_timeout(150)
    check("word saved at once", {"word": "Hetzner", "replacement": None, "isSnippet": False} in saved()["dictionary"])
    check("add card closed", pg.locator("#dict_form_slot .lw-dict-form").count() == 0)
    pg.click("#dict_add")
    inputs = pg.locator("#dict_form_slot input")
    inputs.nth(0).fill("JSON")
    inputs.nth(1).fill("jason")
    pg.click("#dict_form_slot button[type=submit]")
    pg.wait_for_timeout(150)
    check("replacement saved as {heard, word}", {"word": "jason", "replacement": "JSON", "isSnippet": False} in saved()["dictionary"])

    # Tabs, search, sort.
    pg.click("[data-tab=replacements]")
    check("Replacements tab", texts("#dict_rows .lw-dict-row") == ["jason → JSON", "gh → GitHub"], texts("#dict_rows .lw-dict-row"))
    pg.click("[data-tab=words]")
    check("Words tab", texts("#dict_rows .lw-dict-row") == ["Hetzner", "Kubernetes"], texts("#dict_rows .lw-dict-row"))
    pg.click("[data-tab=all]")
    pg.click("#dict_sort")
    check("sort A-Z by word", texts("#dict_rows .lw-dict-row") == ["gh → GitHub", "Hetzner", "jason → JSON", "Kubernetes"], texts("#dict_rows .lw-dict-row"))
    pg.click("#dict_sort")
    pg.click("#dict_search")
    pg.keyboard.type("git")
    check("search", texts("#dict_rows .lw-dict-row") == ["gh → GitHub"])
    pg.keyboard.type("zzz")
    check("no-match empty state", pg.is_visible("#dict_empty") and "Nothing matches" in pg.inner_text("#dict_empty"))
    pg.keyboard.press("Escape")
    check("Escape clears search", len(texts("#dict_rows .lw-dict-row")) == 4 and not pg.is_visible("#dict_filter"))

    # Edit in place, then delete + undo.
    pg.click("#dict_rows .lw-dict-row:has-text('Kubernetes')")
    form = pg.locator("#dict_rows .lw-dict-form input")
    check("edit row prefilled", form.nth(0).input_value() == "Kubernetes" and form.nth(1).input_value() == "")
    form.nth(1).fill("cooper netties")
    pg.click("#dict_rows .lw-dict-form button[type=submit]")
    pg.wait_for_timeout(150)
    d = saved()["dictionary"]
    check("edit saved", {"word": "cooper netties", "replacement": "Kubernetes", "isSnippet": False} in d and not any(e["word"] == "Kubernetes" for e in d))
    pg.hover("#dict_rows .lw-dict-row:has-text('GitHub')")
    pg.click("#dict_rows .lw-dict-row:has-text('GitHub') [aria-label^=Remove]")
    pg.wait_for_timeout(150)
    check("delete saved", not any(e["word"] == "gh" for e in saved()["dictionary"]))
    pg.click("#dict_status .lw-undo")
    pg.wait_for_timeout(150)
    check("undo restores", any(e["word"] == "gh" for e in saved()["dictionary"]))

    # Learn toggle and suggestion accept.
    pg.click("#dict_learn")
    pg.wait_for_timeout(100)
    check("learn toggle saved", saved()["learn_from_edits"] is False)
    pg.click("#dict_learn")
    pg.click("#sugg_list li button:has-text('Add word')")
    pg.wait_for_timeout(150)
    check("suggestion accepted", "Wispr Flow" in texts("#dict_rows .lw-dict-row") and not pg.is_visible("#sugg_list"))

    # Import: merge (header button) then replace all.
    n = len(saved()["dictionary"])
    pg.set_input_files("#dict_file", files=[{"name": "d.json", "mimeType": "application/json", "buffer": json.dumps([{"word": "Imported", "replacement": None, "isSnippet": False}]).encode()}])
    pg.wait_for_timeout(200)
    check("merge import saved", len(saved()["dictionary"]) == n + 1 and "Imported" in texts("#dict_rows .lw-dict-row"))
    pg.set_input_files("#dict_file_replace", files=[{"name": "d.json", "mimeType": "application/json", "buffer": json.dumps(SAMPLE).encode()}])
    pg.wait_for_timeout(200)
    check("replace import saved", saved()["dictionary"] == SAMPLE, saved()["dictionary"])

    # Snippets.
    pg.evaluate("window.__goPage('snippets')")
    pg.wait_for_timeout(150)
    check("snippet rows", len(texts("#snip_rows .lw-dict-row")) == 4)
    pg.click("#snip_add")
    sform = pg.locator("#snip_form_slot")
    sform.locator("input").fill("my calendar link")
    sform.locator("textarea").fill("https://cal.example.com/me/30min")
    pg.screenshot(path=str(out / "dict-snippets-adding-light.png"))
    pg.click("#snip_form_slot button[type=submit]")
    pg.wait_for_timeout(150)
    check("snippet saved", {"word": "my calendar link", "replacement": "https://cal.example.com/me/30min", "isSnippet": True} in saved()["dictionary"])
    check("snippet first", texts("#snip_rows .lw-dict-row")[0].startswith("my calendar link"))
    pg.click("#snip_rows .lw-dict-row:has-text('my sign-off')")
    pg.locator("#snip_rows .lw-dict-form textarea").fill("Thanks,\nBasil")
    pg.keyboard.press("Control+Enter")
    pg.wait_for_timeout(150)
    check("multi-line snippet edit", {"word": "my sign-off", "replacement": "Thanks,\nBasil", "isSnippet": True} in saved()["dictionary"])
    pg.click("#snip_rows .lw-dict-row:has-text('my sign-off')")
    pg.keyboard.press("Escape")
    check("Escape cancels edit", pg.locator("#snip_rows .lw-dict-form").count() == 0)
    pg.close()

    # Screenshots with the artboards' data, light and dark.
    for scheme in ("light", "dark"):
        pg = b.new_page(viewport={"width": 1280, "height": 800}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(400)
        # The mock's state lives in the page, so load the sample data
        # through the UI: dismiss the suggestion, replace-all import.
        pg.evaluate("window.__goPage('dictionary')")
        pg.wait_for_timeout(200)
        pg.click("#sugg_list li button:has-text('Dismiss')")
        replace_with(pg, SAMPLE)
        for page in ("dictionary", "snippets"):
            pg.evaluate("id => window.__goPage(id)", page)
            pg.wait_for_timeout(200)
            pg.screenshot(path=str(out / f"dict-{page}-{scheme}.png"))
            pg.click("#dict_add" if page == "dictionary" else "#snip_add")
            pg.wait_for_timeout(100)
            pg.screenshot(path=str(out / f"dict-{page}-add-{scheme}.png"))
            pg.keyboard.press("Escape")
        pg.evaluate("window.__goPage('dictionary')")
        replace_with(pg, [])
        for page in ("dictionary", "snippets"):
            pg.evaluate("id => window.__goPage(id)", page)
            pg.wait_for_timeout(200)
            pg.screenshot(path=str(out / f"dict-{page}-empty-{scheme}.png"))
        pg.close()
    print("shots in", out.relative_to(HERE.parents[2]))
    print("page errors:", errors)
    b.close()
    if fails or errors:
        sys.exit(1)
