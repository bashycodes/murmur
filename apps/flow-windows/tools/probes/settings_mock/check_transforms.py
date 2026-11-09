"""Headless check of the Transforms page against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_transforms.py [port]
Opens Transforms from the sidebar (between Style and Scratchpad), turns
the opt-in and auto apply on, creates, edits (a clashing shortcut is
refused in the dialog), deletes a transform, resets the built-ins, runs
"Try on sample text", and fires the "view changes" event (Home opens with the
transform row's before / after). Asserts what reached set_prefs. Then
writes shots/transforms-*.png (gitignored) in light and dark.
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


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.on("console", lambda m: m.type == "error" and errors.append(m.text))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(400)
    saved = lambda: pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")["transforms"]
    texts = lambda sel: pg.eval_on_selector_all(sel, "rs => rs.map(r => r.innerText.replace(/\\s+/g, ' ').trim())")

    nav = pg.eval_on_selector_all("[data-page]", "rs => rs.map(r => r.dataset.page)")
    check("sidebar: Style, Transforms, Scratchpad", "transforms" in nav and nav.index("transforms") == nav.index("style") + 1 and nav.index("scratchpad") == nav.index("transforms") + 1, nav)
    pg.click("[data-page=transforms]")
    pg.wait_for_timeout(200)
    check("cards: built-ins + add a transform", texts("#tf_grid .lw-tf-card") == [
        "Win Alt 1 Polish Clearer, tighter wording",
        "Win Alt 2 Prompt Builder Turns a rough ask into a clear prompt",
        "Add a transform Your instruction, your shortcut",
    ], texts("#tf_grid .lw-tf-card"))
    check("view keys Win Alt O", texts("#tf_view kbd") == ["Win", "Alt", "O"] and "shows the last before and after" in pg.inner_text("#tf_view"), texts("#tf_view kbd"))
    check("opt in off by default", not pg.is_checked("#tf_enabled"))
    check("auto apply needs the opt in", pg.is_disabled("#tf_auto") and pg.is_disabled("#tf_auto_pick"))

    pg.click("#tf_enabled")
    pg.wait_for_timeout(100)
    check("opt in saved", saved()["enabled"] is True)
    check("auto toggle usable, pick still off", pg.is_enabled("#tf_auto") and pg.is_disabled("#tf_auto_pick"))
    pg.click("#tf_auto")
    pg.wait_for_timeout(100)
    check("auto apply = polish", saved()["auto_apply"] == "polish", saved()["auto_apply"])
    pg.select_option("#tf_auto_pick", "prompt_engineer")
    pg.wait_for_timeout(100)
    check("auto apply pick saved", saved()["auto_apply"] == "prompt_engineer")

    # Create.
    pg.click("#tf_new")
    check("editor opens", pg.is_visible("#tf_edit_dlg"))
    check("next free shortcut offered", pg.input_value("#tf_shortcut") == "Win+Alt+3", pg.input_value("#tf_shortcut"))
    check("no delete on a new one", pg.is_hidden("#tf_edit_delete"))
    pg.click("#tf_edit_save")
    check("empty name refused", "name" in pg.inner_text("#tf_edit_err"))
    pg.fill("#tf_name", "Make it friendly!")
    pg.click("#tf_edit_save")
    check("empty instruction refused", "instruction" in pg.inner_text("#tf_edit_err"))
    pg.fill("#tf_desc", "Warmer, same length")
    pg.fill("#tf_instruction", "Rewrite the text so it sounds warm and friendly.")
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    items = saved()["items"]
    check("created with a slug id", len(items) == 3 and items[2]["transform"]["id"] == "make-it-friendly" and items[2]["shortcut"] == "Win+Alt+3", items[-1])
    check("dialog closed", not pg.is_visible("#tf_edit_dlg"))
    check("card shown", "Make it friendly!" in pg.inner_text("#tf_grid"))

    # Edit: a clashing shortcut is refused and the dialog stays open.
    pg.click("#tf_grid .lw-tf-card[data-id=make-it-friendly]")
    check("editor filled", pg.input_value("#tf_name") == "Make it friendly!" and pg.is_visible("#tf_edit_delete"))
    pg.fill("#tf_shortcut", "Win+Alt+1")
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    check("clash refused in the dialog", pg.is_visible("#tf_edit_dlg") and "twice" in pg.inner_text("#tf_edit_err"), pg.inner_text("#tf_edit_err"))
    check("nothing saved on a clash", saved()["items"][2]["shortcut"] == "Win+Alt+3")
    pg.fill("#tf_shortcut", "")
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    check("shortcut cleared", saved()["items"][2]["shortcut"] is None)
    check("card says no shortcut", "No shortcut" in pg.inner_text("#tf_grid .lw-tf-card[data-id=make-it-friendly]"))

    # Delete.
    pg.click("#tf_grid .lw-tf-card[data-id=make-it-friendly]")
    pg.click("#tf_edit_delete")
    pg.wait_for_timeout(150)
    check("deleted", [i["transform"]["id"] for i in saved()["items"]] == ["polish", "prompt_engineer"])

    # Edit a built-in, then Reset to defaults puts it back and keeps custom ones.
    pg.click("#tf_grid .lw-tf-card[data-id=polish]")
    check("built-in hint", pg.is_visible("#tf_builtin_hint"))
    pg.fill("#tf_desc", "Mine now")
    pg.fill("#tf_shortcut", "Win+Alt+P")
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    pg.click("#tf_new")
    pg.fill("#tf_name", "Shorter")
    pg.fill("#tf_instruction", "Make it half as long.")
    pg.fill("#tf_shortcut", "Win+Alt+1")
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    check("edits saved", saved()["items"][0]["transform"]["description"] == "Mine now" and saved()["items"][2]["shortcut"] == "Win+Alt+1")
    pg.click("#tf_reset")
    pg.click("#tf_reset_ok")
    pg.wait_for_timeout(150)
    items = saved()["items"]
    check("reset: built-in wording back, custom kept", items[0]["transform"]["description"] == "Clearer, tighter wording" and items[2]["transform"]["name"] == "Shorter", items)
    check("reset: built-in loses a shortcut a custom took", items[0]["shortcut"] is None and items[1]["shortcut"] == "Win+Alt+2")
    check("reset shown", "Win Alt 1 Shorter" in texts("#tf_grid .lw-tf-card")[2])

    # Try it out.
    pg.click("#tf_try")
    check("try dialog lists transforms", texts("#tf_try_pick option") == ["Polish", "Prompt Builder", "Shorter"])
    pg.click("#tf_try_run")
    pg.wait_for_timeout(150)
    check("try result shown", pg.inner_text("#tf_try_out").startswith("Polish: Hey so"), pg.inner_text("#tf_try_out"))
    pg.evaluate("window.__failTransform = true")
    pg.click("#tf_try_run")
    pg.wait_for_timeout(150)
    check("try error shown", "Transforms need AI" in pg.inner_text("#tf_try_out"))
    pg.click("#tf_try_close")

    # View changes (Win+Alt+O): Home with the row's before / after open.
    pg.evaluate("window.__emit('transforms:view', 6)")
    pg.wait_for_timeout(250)
    active = pg.evaluate("document.querySelector('[data-page=home]').getAttribute('aria-current') || document.querySelector('[data-page=home]').className")
    check("home shown", pg.is_visible("#hist_list"), active)
    check("transform badge", "Transform: Polish" in pg.inner_text("#hist_list .lw-hist-row[data-id='6']"))
    check("before / after open", pg.is_visible("#hist_list .lw-hist-row[data-id='6'] .lw-tf-change"))
    check("diff marks removed words", pg.locator("#hist_list .lw-hist-row[data-id='6'] .lw-tf-change del").count() > 0)
    pg.screenshot(path=str(out / "transforms-view-light.png"))
    pg.evaluate("window.__emit('transforms:view', null)")
    pg.wait_for_timeout(250)
    check("no transform yet", "No transform yet" in pg.inner_text("#hist_status"))
    pg.close()

    for scheme in ("light", "dark"):
        pg = b.new_page(viewport={"width": 1280, "height": 800}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(400)
        pg.click("[data-page=transforms]")
        pg.click("#tf_enabled")
        pg.wait_for_timeout(200)
        pg.screenshot(path=str(out / f"transforms-{scheme}.png"))
        pg.click("#tf_grid .lw-tf-card[data-id=polish]")
        pg.wait_for_timeout(100)
        pg.screenshot(path=str(out / f"transforms-edit-{scheme}.png"))
        pg.keyboard.press("Escape")
        pg.close()
    print("shots in", out.relative_to(HERE.parents[2]))
    print("page errors:", errors)
    b.close()
    if fails or errors:
        sys.exit(1)
