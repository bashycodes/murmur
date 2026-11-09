"""Headless check of models from the server's library (D41) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_server_models.py [port]
With a server set (?server=1) Settings -> Speech to text lists the server's
library under "From your server": every model with its kind and size, a
Download / Use button for the ones this app can load and the reason for the
others. The catalogue rows download from the server ("Download models
from: Your server", D79); one the library lacks says "Not on your server". Covers: a custom whisper model (confirm dialog says "your server" and
the server-listed fingerprint, `server_model_download`, prefs point at it,
"in use"), a catalogue copy (built-in fingerprint, `model_download_start`
with the catalogue id), a custom Hebrew model (the Hebrew row says it uses
it), a language-ID model, cancel, an unreadable library (?server=err), and
no server (group hidden, Hugging Face downloads back). Writes
shots/server-*.png (gitignored).
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
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def calls(pg, cmd):
    return pg.evaluate("(c) => window.__mockCalls.filter(x => x[0] === c).map(x => x[1])", cmd)


def row(pg, mid):
    return pg.locator(f".lw-srv-model[data-srv-id='{mid}']")


def open_page(b, scheme, query):
    pg = b.new_page(viewport={"width": 1280, "height": 1000}, color_scheme=scheme)
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html{query}")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
    pg.evaluate("window.__goPage('settings-speech')")
    pg.wait_for_timeout(300)
    return pg


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = open_page(b, scheme, "?server=1")
        check(tag + "group shown with a server set", pg.is_visible("#srv_group"))
        check(tag + "seven library models listed", pg.locator(".lw-srv-model").count() == 7, str(pg.locator(".lw-srv-model").count()))
        txt = row(pg, "whisper-large-he").inner_text()
        check(tag + "custom row: kind, size, engine", "Speech to text · 1080 MB · whisper" in txt and "Download 1080 MB…" in txt, txt)
        txt = row(pg, "silero").inner_text()
        check(tag + "unusable: reason, no button", "Can't be used here: this app has no slot for a voice-activity model." in txt and row(pg, "silero").locator("button").count() == 0, txt)
        txt = row(pg, "ct2-medium").inner_text()
        check(tag + "unknown engine: reason", "ctranslate2" in txt and row(pg, "ct2-medium").locator("button").count() == 0, txt)
        txt = row(pg, "whisper-base").inner_text()
        check(tag + "catalogue copy already here: in use", "same as the built-in model" in txt and "in use" in txt, txt)
        check(tag + "cleanup + LID + Hebrew kinds", "AI cleanup" in row(pg, "qwen15").inner_text() and "Language detection" in row(pg, "tiny-lid").inner_text() and "Hebrew · he" in row(pg, "zipformer-he-v2").inner_text())
        # Catalogue rows: downloaded from the server (D79), not Hugging Face.
        cat = pg.evaluate("window.__TAURI__.core.invoke('model_catalog')")
        check(tag + "catalogue: every row from the server", all(c["available"] and c.get("from_server") for c in cat))
        pg.select_option("#dl_model", "whisper-small-q5_1")
        pg.wait_for_timeout(100)
        st = pg.inner_text("#dl_status")
        check(tag + "speech row: not on the server, no Download", not pg.is_visible("#dl_start") and pg.is_visible("#dl_import") and pg.is_visible("#dl_missing") and "doesn't have" in st, st)
        if scheme == "light":
            pg.locator("#srv_group").screenshot(path=str(out / "server-list.png"))

        # --- A custom whisper model.
        row(pg, "whisper-large-he").locator("button").click()
        pg.wait_for_timeout(100)
        check(tag + "confirm dialog first", pg.is_visible("#dl_confirm") and calls(pg, "server_model_download") == [])
        check(tag + "dialog: from your server, its fingerprint", pg.inner_text("#dl_from").startswith("Your server") and "your server lists" in pg.inner_text("#dl_check"), pg.inner_text("#dl_check"))
        check(tag + "dialog: saved under models\\server", pg.inner_text("#dl_dest").endswith("server\\whisper-large-he\\ggml-large-he.bin"), pg.inner_text("#dl_dest"))
        if scheme == "light":
            pg.locator("#dl_confirm").screenshot(path=str(out / "server-confirm.png"))
        pg.click("#dl_cancel")
        pg.wait_for_timeout(100)
        check(tag + "dialog cancel: nothing starts", calls(pg, "server_model_download") == [])
        row(pg, "whisper-large-he").locator("button").click()
        pg.wait_for_timeout(100)
        pg.click("#dl_ok")
        pg.wait_for_timeout(60)
        txt = row(pg, "whisper-large-he").inner_text()
        check(tag + "progress from your server", "from your server" in txt or "fingerprint" in txt, txt)
        pg.wait_for_timeout(900)
        after = prefs(pg)
        check(tag + "custom: server_model_download, never model_download_start", calls(pg, "server_model_download") == [{"id": "whisper-large-he"}] and calls(pg, "model_download_start") == [])
        check(tag + "custom: engine points at it", after["asr_engine"] == "whisper" and after["model_path"].endswith("server\\whisper-large-he\\ggml-large-he.bin"), str(after["model_path"]))
        txt = row(pg, "whisper-large-he").inner_text()
        check(tag + "custom: in use", "Downloaded and in use for the model on this PC (Whisper)." in txt, txt)
        txt = row(pg, "whisper-base").inner_text()
        check(tag + "the old one offers Use", "Press Use" in txt and row(pg, "whisper-base").locator("button").inner_text() == "Use", txt)

        # --- A catalogue copy (AI cleanup): the built-in fingerprint.
        row(pg, "qwen15").locator("button").click()
        pg.wait_for_timeout(100)
        check(tag + "catalogue copy: built-in fingerprint", "built into this version" in pg.inner_text("#dl_check"))
        pg.click("#dl_ok")
        pg.wait_for_timeout(900)
        check(tag + "catalogue copy: model_download_start with the catalogue id", calls(pg, "model_download_start") == [{"id": "qwen2.5-1.5b-instruct-q4_k_m"}], str(calls(pg, "model_download_start")))
        check(tag + "catalogue copy: cleanup on with it", prefs(pg)["llm_enabled"] is True)

        # --- Cancel.
        row(pg, "tiny-lid").locator("button").click()
        pg.wait_for_timeout(100)
        pg.evaluate("window.__dlTickMs = 200")
        pg.click("#dl_ok")
        pg.wait_for_timeout(80)
        row(pg, "tiny-lid").locator("button", has_text="Cancel").click()
        pg.wait_for_timeout(500)
        txt = row(pg, "tiny-lid").inner_text()
        check(tag + "cancel: nothing kept, Download again", "cancelled" in txt and "Download" in row(pg, "tiny-lid").locator("button").inner_text(), txt)
        check(tag + "cancel: prefs untouched", not prefs(pg).get("lid_model_path"))
        pg.evaluate("window.__dlTickMs = 30")

        # --- A custom Hebrew model.
        row(pg, "zipformer-he-v2").locator("button").click()
        pg.wait_for_timeout(100)
        pg.click("#dl_ok")
        pg.wait_for_timeout(900)
        after = prefs(pg)
        check(tag + "Hebrew: route points at the server folder", after["hebrew_model"] == "zipformer" and after["hebrew_model_dir"].endswith("server\\zipformer-he-v2"), str(after.get("hebrew_model_dir")))
        st = pg.inner_text("#he_status")
        check(tag + "Hebrew row: uses it", "from your server" in st, st)
        check(tag + "no page errors so far", not errors, "; ".join(errors[:3]))
        pg.close()

    # --- Library can't be read.
    pg = open_page(b, "light", "?server=err")
    check("error: shown with the reason", pg.is_visible("#srv_group") and "refused the key" in pg.inner_text("#srv_note"), pg.inner_text("#srv_note"))
    check("error: no rows", pg.locator(".lw-srv-model").count() == 0)
    pg.close()

    # --- No server: Hugging Face as before, no group.
    pg = open_page(b, "light", "")
    check("no server: group hidden", not pg.is_visible("#srv_group"))
    check("no server: server_models not even asked", calls(pg, "server_models") == [])
    pg.select_option("#dl_model", "whisper-small-q5_1")
    pg.wait_for_timeout(100)
    check("no server: Download from Hugging Face is back", pg.is_visible("#dl_start"))
    pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
