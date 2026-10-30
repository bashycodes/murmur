"""Headless check of "press the keys" shortcut recording against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_keycapture.py [port]
Settings → General: Change starts a recording (hotkey_capture_start,
"Press keys… Esc to cancel", Cancel, Type instead); the simulated hook
event (window.__capture) saves the chord through set_prefs; Esc and the
10 s timeout keep the old one; a chord the validation refuses shows the
error; Type instead falls back to the text field; leaving the window stops
the recording; no hook = an error and the old value. Transforms: "Press
keys" fills the shortcut field. Writes shots/keycapture-*.png (gitignored).
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
    shortcuts = lambda: pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")["shortcuts"]
    calls = lambda c: pg.evaluate(f"window.__mockCalls.filter(x => x[0] === '{c}').length")
    keys = lambda id: pg.eval_on_selector_all(f"#{id}_view kbd", "ks => ks.map(k => k.textContent)")
    box = lambda id: f".lw-edit[data-edit={id}]"

    pg.evaluate("window.__goPage('settings-general')")
    pg.wait_for_timeout(200)
    check("ptt keycaps", keys("ptt") == ["Ctrl", "Win"], keys("ptt"))

    # Change → recording.
    pg.click(f"{box('ptt')} .lw-edit-btn")
    pg.wait_for_timeout(50)
    check("capture started", calls("hotkey_capture_start") == 1 and pg.evaluate("window.__capturing") is True)
    check("recording text", pg.inner_text("#ptt_view") == "Press keys… Esc to cancel", pg.inner_text("#ptt_view"))
    check("button says Cancel", pg.inner_text(f"{box('ptt')} .lw-edit-btn") == "Cancel")
    check("type instead offered", pg.is_visible(f"{box('ptt')} .lw-edit-type"))
    check("text field stays hidden", pg.is_hidden("#ptt"))
    pg.screenshot(path=str(out / "keycapture-recording.png"))

    # The hook reports Right Alt → saved.
    pg.evaluate("window.__capture('Right Alt')")
    pg.wait_for_timeout(200)
    check("Right Alt saved", shortcuts()["ptt"] == "Right Alt", shortcuts()["ptt"])
    check("keycaps show it", keys("ptt") == ["Right Alt"], keys("ptt"))
    check("back to Change", pg.inner_text(f"{box('ptt')} .lw-edit-btn") == "Change" and pg.is_hidden(f"{box('ptt')} .lw-edit-type"))
    check("saved status", "Saved" in pg.inner_text("#status"), pg.inner_text("#status"))

    # Esc (cancelled) keeps the old one.
    pg.click(f"{box('hands_free')} .lw-edit-btn")
    pg.evaluate("window.__capture(null)")
    pg.wait_for_timeout(150)
    check("Esc keeps hands-free", shortcuts()["hands_free"] == "Ctrl+Win+Space" and keys("hands_free") == ["Ctrl", "Win", "Space"], keys("hands_free"))

    # Timeout keeps it too, and says so.
    pg.click(f"{box('hands_free')} .lw-edit-btn")
    pg.evaluate("window.__capture(null, 'timeout')")
    pg.wait_for_timeout(150)
    check("timeout keeps hands-free", shortcuts()["hands_free"] == "Ctrl+Win+Space")
    check("timeout explained", "No keys pressed" in pg.inner_text("#status"), pg.inner_text("#status"))

    # Cancel stops the hook's recording.
    pg.click(f"{box('command')} .lw-edit-btn")
    pg.click(f"{box('command')} .lw-edit-btn")
    pg.wait_for_timeout(50)
    check("Cancel stops capture", pg.evaluate("window.__capturing") is False and calls("hotkey_capture_stop") >= 1)
    check("Cancel shows the old keys", keys("command") == ["Ctrl", "Win", "Alt"], keys("command"))

    # Starting another field's recording ends the first.
    pg.click(f"{box('paste_last')} .lw-edit-btn")
    pg.click(f"{box('copy_last')} .lw-edit-btn")
    pg.wait_for_timeout(50)
    check("one recording at a time", pg.locator(".lw-edit.is-recording").count() == 1 and keys("paste_last") == ["Alt", "Shift", "Z"])
    pg.evaluate("window.__capture('Ctrl+Alt+C')")
    pg.wait_for_timeout(150)
    check("second field saved", shortcuts()["copy_last"] == "Ctrl+Alt+C" and shortcuts()["paste_last"] == "Alt+Shift+Z", shortcuts())

    # Type instead: the text field, as before.
    pg.click(f"{box('scratchpad')} .lw-edit-btn")
    pg.click(f"{box('scratchpad')} .lw-edit-type")
    pg.wait_for_timeout(50)
    check("type instead stops capture", pg.evaluate("window.__capturing") is False)
    check("text field shown", pg.is_visible("#scratchpad") and pg.is_visible("#keys_help"))
    pg.fill("#scratchpad", "Ctrl+Alt+M")
    pg.press("#scratchpad", "Enter")
    pg.wait_for_timeout(150)
    check("typed chord saved", shortcuts()["scratchpad"] == "Ctrl+Alt+M", shortcuts()["scratchpad"])
    check("keycaps after typing", keys("scratchpad") == ["Ctrl", "Alt", "M"], keys("scratchpad"))

    # Leaving the window stops recording.
    pg.click(f"{box('new_note')} .lw-edit-btn")
    pg.evaluate("window.dispatchEvent(new Event('blur'))")
    pg.wait_for_timeout(50)
    check("blur stops capture", pg.evaluate("window.__capturing") is False and pg.locator(".lw-edit.is-recording").count() == 0)

    # No hook: an error, old value kept.
    pg.evaluate("window.__noHook = true")
    pg.click(f"{box('cancel')} .lw-edit-btn")
    pg.wait_for_timeout(100)
    check("no hook explained", "type the shortcut" in pg.inner_text("#status"), pg.inner_text("#status"))
    check("no hook keeps Esc", keys("cancel") == ["Esc"] and pg.locator(".lw-edit.is-recording").count() == 0)
    pg.evaluate("window.__noHook = false")

    # Transforms: Press keys fills the field; Cancel puts the old one back.
    pg.evaluate("window.__goPage('transforms')")
    pg.wait_for_timeout(200)
    pg.click("#tf_grid .lw-tf-card[data-id=polish]")
    check("transform shortcut", pg.input_value("#tf_shortcut") == "Win+Alt+1")
    pg.click("#tf_shortcut_rec")
    pg.wait_for_timeout(50)
    check("transform recording", pg.evaluate("window.__capturing") is True and "Press keys" in pg.get_attribute("#tf_shortcut", "placeholder") and pg.inner_text("#tf_shortcut_rec") == "Stop")
    pg.screenshot(path=str(out / "keycapture-transform.png"))
    pg.click("#tf_shortcut_rec")
    pg.wait_for_timeout(50)
    check("transform cancel restores", pg.input_value("#tf_shortcut") == "Win+Alt+1" and pg.evaluate("window.__capturing") is False)
    pg.click("#tf_shortcut_rec")
    pg.evaluate("window.__capture('Win+Alt+P')")
    pg.wait_for_timeout(100)
    check("transform chord filled", pg.input_value("#tf_shortcut") == "Win+Alt+P", pg.input_value("#tf_shortcut"))
    pg.click("#tf_edit_save")
    pg.wait_for_timeout(150)
    tf = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")["transforms"]["items"][0]["shortcut"]
    check("transform chord saved", tf == "Win+Alt+P", tf)
    pg.click("#tf_grid .lw-tf-card[data-id=polish]")
    pg.click("#tf_shortcut_rec")
    pg.click("#tf_edit_cancel")
    pg.wait_for_timeout(50)
    check("closing the dialog stops capture", pg.evaluate("window.__capturing") is False)

    print("page errors:", errors)
    b.close()
    if fails or errors:
        sys.exit(1)
