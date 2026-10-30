"""Headless check of Hebrew (D33, task H4) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_hebrew.py [port]
Settings -> General: the language picker offers Hebrew; choosing it with no
Hebrew model downloaded shows the hint that links to Speech to text. There:
the Hebrew model row (Zipformer recommended / Parakeet Hebrew / None), the
CC-BY-4.0 credit, Download -> confirm dialog -> done makes it the Hebrew
route and leaves the default model alone; then the hint is gone. Also the
"Other..." codes field. Writes shots/he-*.png (gitignored).
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


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        pg.evaluate("window.__goPage('settings-general')")
        pg.wait_for_timeout(200)
        opts = pg.eval_on_selector_all("#language_pick option", "os => os.map(o => [o.value, o.textContent])")
        check(tag + "picker offers Hebrew", ["he", "Hebrew"] in opts and opts[0] == ["", "Detect automatically"], str(opts))
        check(tag + "auto-detect: no Hebrew hint", pg.input_value("#language_pick") == "" and not pg.is_visible("#languages_hebrew"))
        pg.select_option("#language_pick", "he")
        pg.wait_for_timeout(200)
        check(tag + "Hebrew saved as the one language", prefs(pg)["languages"] == ["he"], str(prefs(pg)["languages"]))
        check(tag + "no Hebrew model -> hint", pg.is_visible("#languages_hebrew") and "No Hebrew model" in pg.inner_text("#languages_hebrew"))
        check(tag + "codes field stays hidden", not pg.is_visible("#languages_other"))
        if scheme == "light":
            pg.locator(".lw-section:has(#language_pick)").screenshot(path=str(out / "he-general-hint.png"))
        pg.click("#languages_hebrew [data-go=settings-speech]")
        pg.wait_for_timeout(200)
        check(tag + "hint links to Speech to text", pg.is_visible("#he_row"))
        check(tag + "Zipformer is the default choice", pg.input_value("#hebrew_model") == "zipformer")
        st = pg.inner_text("#he_status")
        check(tag + "row says it is not downloaded", "Not downloaded" in st and "err" in pg.get_attribute("#he_status", "class"), st)
        check(tag + "download button with size", pg.is_visible("#he_download") and pg.inner_text("#he_download") == "Download 104 MB…", pg.inner_text("#he_download"))
        credit = pg.inner_text("#he_credit")
        check(tag + "CC-BY-4.0 credit", "CC-BY-4.0" in credit and "ivrit.ai" in credit and pg.is_visible("#he_credit [data-open-url*='zipformer2-66m']"), credit)
        dl = pg.eval_on_selector_all("#dl_model option", "os => os.map(o => o.value)")
        check(tag + "default-model download list has no Hebrew", len(dl) == 3 and not any("he" in v.split("-") for v in dl), str(dl))
        if scheme == "light":
            pg.locator("#he_row").screenshot(path=str(out / "he-speech-missing.png"))
        before = prefs(pg)
        pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
        pg.click("#he_download")
        pg.wait_for_timeout(100)
        check(tag + "confirm dialog first", pg.is_visible("#dl_confirm") and "Download Zipformer Hebrew?" in pg.inner_text("#dl_title"))
        check(tag + "confirm: 4 model files + the language-ID model", "104 MB in 5 files" in pg.inner_text("#dl_size") and "zipformer2-66m-streaming-he" in pg.inner_text("#dl_from") and pg.inner_text("#dl_dest").endswith("zipformer2-66m-streaming-he-int8"), pg.inner_text("#dl_size"))
        pg.click("#dl_ok")
        pg.wait_for_timeout(800)
        after = prefs(pg)
        check(tag + "done -> Hebrew route, default untouched", after["hebrew_model"] == "zipformer" and after["asr_engine"] == before["asr_engine"] and after["model_path"] == before["model_path"], str(after["hebrew_model"]))
        st = pg.inner_text("#he_status")
        check(tag + "Hebrew model ready", "Ready for Hebrew" in st and not pg.is_visible("#he_download"), st)
        if scheme == "light":
            pg.locator("#he_row").screenshot(path=str(out / "he-speech-ready.png"))
        pg.evaluate("window.__goPage('settings-general')")
        pg.wait_for_timeout(200)
        check(tag + "hint gone once downloaded", not pg.is_visible("#languages_hebrew"))
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(100)
        pg.select_option("#hebrew_model", "parakeet_he")
        pg.wait_for_timeout(300)
        check(tag + "Parakeet Hebrew saved", prefs(pg)["hebrew_model"] == "parakeet_he")
        check(tag + "Parakeet Hebrew offers its download", pg.inner_text("#he_download") == "Download 673 MB…" and "krut42" in pg.inner_text("#he_credit"), pg.inner_text("#he_download"))
        pg.select_option("#hebrew_model", "none")
        pg.wait_for_timeout(300)
        st = pg.inner_text("#he_status")
        check(tag + "None: Hebrew uses the default model", prefs(pg)["hebrew_model"] == "none" and "model above" in st and not pg.is_visible("#he_download") and not pg.is_visible("#he_credit"), st)
        pg.evaluate("window.__goPage('settings-general')")
        pg.wait_for_timeout(100)
        check(tag + "hint back with no Hebrew model", pg.is_visible("#languages_hebrew"))
        # Other languages: the codes field.
        pg.select_option("#language_pick", "other")
        pg.wait_for_timeout(100)
        check(tag + "Other opens the codes field", pg.is_visible("#languages") and pg.is_visible("#languages_other"))
        pg.fill("#languages", "en, de")
        pg.press("#languages", "Enter")
        pg.wait_for_timeout(200)
        check(tag + "codes saved", prefs(pg)["languages"] == ["en", "de"] and pg.input_value("#language_pick") == "other", str(prefs(pg)["languages"]))
        check(tag + "no Hebrew hint for other languages", not pg.is_visible("#languages_hebrew"))
        pg.select_option("#language_pick", "")
        pg.wait_for_timeout(200)
        check(tag + "back to detect", prefs(pg)["languages"] == [] and not pg.is_visible("#languages_other") and "Detected automatically" in pg.inner_text("#languages_desc"))
        pg.close()
    # Zipformer Hebrew downloaded before the language-ID model came with it.
    pg = b.new_page(viewport={"width": 1280, "height": 900})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html?he=nolid")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__goPage('settings-speech')")
    pg.wait_for_timeout(200)
    st = pg.inner_text("#he_status")
    check("[nolid] ready, but auto-detect asks for the language-ID model", "Ready for Hebrew" in st and "Auto-detect needs a 32 MB language-ID model" in st, st)
    check("[nolid] download offers only the missing file", pg.is_visible("#he_download") and pg.inner_text("#he_download") == "Download 32 MB…", pg.inner_text("#he_download"))
    pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
    pg.click("#he_download")
    pg.wait_for_timeout(100)
    pg.click("#dl_ok")
    pg.wait_for_timeout(800)
    st = pg.inner_text("#he_status")
    check("[nolid] complete after the download", "Ready for Hebrew" in st and "Auto-detect" not in st and not pg.is_visible("#he_download"), st)
    pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
