"""Headless check of manual model import (folder / zip / file) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_model_import.py [port]
"Import from file..." sits next to each downloadable model: Settings -> Speech
to text (default model row and Hebrew row) and AI cleanup. It opens the import
dialog (the files the model needs, the language-ID extra marked optional, a
folder or a .zip / model file picker), then `model_import_start` with the
picked path; never `model_download_start`. Covers: a whisper file import
pointing the engine at it, a cleanup model import turning local cleanup on, a
fingerprint mismatch (error, nothing installed, prefs untouched), a custom
file refused, the dialog / picker cancelled (no import), and a client-only
build (?nodl=1) where Download is gone but Import stays, including a Hebrew
import without the language-ID file completed later by importing that file.
Writes shots/import-*.png (gitignored).
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []
Q15 = "qwen2.5-1.5b-instruct-q4_k_m"
Q05 = "qwen2.5-0.5b-instruct-q4_k_m"


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def calls(pg, cmd):
    return pg.evaluate("(c) => window.__mockCalls.filter(x => x[0] === c).map(x => x[1])", cmd)


def pick(pg, path):
    pg.evaluate("(p) => { window.__pickResult = p; }", path)


def open_page(b, scheme, query=""):
    pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html{query}")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
    return pg


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = open_page(b, scheme)
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)
        # --- Speech to text: Whisper small from its .bin file.
        pg.select_option("#dl_model", "whisper-small-q5_1")
        pg.wait_for_timeout(100)
        check(tag + "speech row: Download and Import side by side", pg.is_visible("#dl_start") and pg.is_visible("#dl_import") and pg.inner_text("#dl_import") == "Import from file…")
        pg.click("#dl_import")
        pg.wait_for_timeout(100)
        check(tag + "import dialog first", pg.is_visible("#imp_dialog") and pg.inner_text("#imp_title") == "Import Whisper small")
        files = pg.inner_text("#imp_files")
        check(tag + "dialog lists the pinned file and size", "ggml-small-q5_1.bin · 190 MB" in files, files)
        check(tag + "single-file model: zip or model file", pg.inner_text("#imp_file") == "Choose .zip or model file…" and pg.is_visible("#imp_folder"))
        check(tag + "says nothing is downloaded", "Nothing is downloaded" in pg.inner_text("#imp_dialog"))
        if scheme == "light":
            pg.locator("#imp_dialog").screenshot(path=str(out / "import-dialog-whisper.png"))
        # Cancel in the dialog: no picker, no import.
        pg.click("#imp_cancel")
        pg.wait_for_timeout(100)
        check(tag + "dialog cancel: no picker", not pg.is_visible("#imp_dialog") and calls(pg, "pick_path") == [])
        # Picker cancelled: no import.
        pick(pg, None)
        pg.click("#dl_import")
        pg.wait_for_timeout(100)
        pg.click("#imp_file")
        pg.wait_for_timeout(200)
        check(tag + "picker cancel: no import", calls(pg, "model_import_start") == [] and len(calls(pg, "pick_path")) == 1)
        f = calls(pg, "pick_path")[0]
        check(tag + "file picker offers the .bin and .zip", f["kind"] == "file" and f["filters"][0]["pattern"] == "*.bin;*.zip", str(f))
        # The real import.
        before = prefs(pg)
        pick(pg, "D:\\usb\\ggml-small-q5_1.bin")
        pg.click("#dl_import")
        pg.wait_for_timeout(100)
        pg.click("#imp_file")
        pg.wait_for_timeout(120)
        st = pg.inner_text("#dl_status")
        check(tag + "progress copies from this PC", "from this PC" in st or "fingerprint" in st, st)
        pg.wait_for_timeout(900)
        after = prefs(pg)
        check(tag + "import started with the picked path", calls(pg, "model_import_start") == [{"id": "whisper-small-q5_1", "path": "D:\\usb\\ggml-small-q5_1.bin"}], str(calls(pg, "model_import_start")))
        check(tag + "no network: never model_download_start", calls(pg, "model_download_start") == [])
        check(tag + "imported -> engine uses it", after["asr_engine"] == "whisper" and (after["model_path"] or "").endswith("ggml-small-q5_1.bin"), str((after["asr_engine"], after["model_path"])))
        check(tag + "status: imported and checked", "Imported and checked" in pg.inner_text("#dl_status"), pg.inner_text("#dl_status"))
        label = pg.eval_on_selector("#dl_model option[value='whisper-small-q5_1']", "o => o.textContent")
        check(tag + "list marks it downloaded", "downloaded" in label, label)
        # --- Fingerprint mismatch on Parakeet (folder model, .zip).
        pg.select_option("#dl_model", "parakeet-tdt-0.6b-v2-int8")
        pg.wait_for_timeout(100)
        pg.click("#dl_import")
        pg.wait_for_timeout(100)
        check(tag + "folder model: .zip only for files", pg.inner_text("#imp_file") == "Choose .zip…" and "encoder.int8.onnx" in pg.inner_text("#imp_files") and "tokens.txt" in pg.inner_text("#imp_files"))
        pick(pg, "D:\\usb\\bad\\parakeet.zip")
        pg.click("#imp_file")
        pg.wait_for_timeout(900)
        st = pg.inner_text("#dl_status")
        check(tag + "mismatch: clear error, nothing installed", "Import failed" in st and "fingerprint" in st and "Nothing was installed" in st and "err" in pg.get_attribute("#dl_status", "class"), st)
        p2 = prefs(pg)
        check(tag + "mismatch: prefs untouched", (p2["asr_engine"], p2["parakeet_model_dir"]) == (after["asr_engine"], after["parakeet_model_dir"]))
        label = pg.eval_on_selector("#dl_model option[value='parakeet-tdt-0.6b-v2-int8']", "o => o.textContent")
        check(tag + "mismatch: not marked downloaded", "downloaded" not in label, label)
        # Folder picker for a folder model.
        pick(pg, "D:\\usb\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8")
        pg.click("#dl_import")
        pg.wait_for_timeout(100)
        pg.click("#imp_folder")
        pg.wait_for_timeout(900)
        f = calls(pg, "pick_path")[-1]
        check(tag + "folder picker, no filters", f["kind"] == "folder" and f["filters"] is None, str(f))
        p3 = prefs(pg)
        check(tag + "parakeet folder import -> engine uses it", p3["asr_engine"] == "parakeet" and p3["parakeet_model_dir"].endswith("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"), str(p3["parakeet_model_dir"]))
        # --- AI cleanup.
        pg.evaluate("window.__goPage('settings-cleanup')")
        pg.wait_for_timeout(300)
        pg.select_option("#llm_dl_model", Q05)
        pg.wait_for_timeout(100)
        check(tag + "cleanup row: Import next to Download", pg.is_visible("#llm_dl_start") and pg.is_visible("#llm_dl_import"))
        if scheme == "light":
            pg.locator("#llm_dl_row").screenshot(path=str(out / "import-llm-row.png"))
        pick(pg, "D:\\usb\\custom-finetune.gguf")
        pg.click("#llm_dl_import")
        pg.wait_for_timeout(100)
        f_btn = pg.inner_text("#imp_file")
        pg.click("#imp_file")
        pg.wait_for_timeout(300)
        st = pg.inner_text("#llm_dl_status")
        check(tag + "custom file refused", "none of the files" in st and "custom models" in st and pg.is_visible("#llm_dl_import"), st)
        check(tag + "gguf filter", calls(pg, "pick_path")[-1]["filters"][0]["pattern"] == "*.gguf;*.zip" and f_btn == "Choose .zip or model file…")
        pg.select_option("#llm_dl_model", Q15)
        pg.wait_for_timeout(100)
        pg.evaluate("(async () => { const t = window.__TAURI__.core; const p = await t.invoke('get_prefs'); p.llm_enabled = false; await t.invoke('set_prefs', { prefs: p }); })()")
        pg.wait_for_timeout(100)
        pick(pg, "D:\\usb\\models")
        pg.click("#llm_dl_import")
        pg.wait_for_timeout(100)
        pg.click("#imp_folder")
        pg.wait_for_timeout(900)
        p4 = prefs(pg)
        check(tag + "cleanup import -> local cleanup on with it", p4["llm_enabled"] and p4["llm_model_path"].endswith(Q15 + ".gguf"), str((p4["llm_enabled"], p4["llm_model_path"])))
        check(tag + "cleanup: in use, no Import/Download", "in use" in pg.inner_text("#llm_dl_status") and not pg.is_visible("#llm_dl_import") and not pg.is_visible("#llm_dl_start"), pg.inner_text("#llm_dl_status"))
        pg.close()

        # --- Client-only build: no downloader, import still works.
        tag = f"[nodl {scheme}] "
        pg = open_page(b, scheme, "?nodl=1")
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)
        check(tag + "speech row shown with Import only", pg.is_visible("#dl_row") and pg.is_visible("#dl_import") and not pg.is_visible("#dl_start"))
        st = pg.inner_text("#he_status")
        check(tag + "Hebrew: Import, no Download", pg.is_visible("#he_import") and not pg.is_visible("#he_download") and "Import from file" in st, st)
        pg.click("#he_import")
        pg.wait_for_timeout(100)
        files = pg.inner_text("#imp_files")
        check(tag + "Hebrew dialog: 4 files + optional language-ID", "encoder.int8.onnx" in files and "ggml-tiny-q5_1.bin · 32 MB · optional, for auto-detect" in files, files)
        if scheme == "light":
            pg.locator("#imp_dialog").screenshot(path=str(out / "import-dialog-hebrew.png"))
        before = prefs(pg)
        pick(pg, "D:\\usb\\zipformer-nolid.zip")
        pg.click("#imp_file")
        pg.wait_for_timeout(900)
        after = prefs(pg)
        st = pg.inner_text("#he_status")
        check(tag + "Hebrew imported -> Hebrew route, default untouched", after["hebrew_model"] == "zipformer" and after["asr_engine"] == before["asr_engine"] and after["model_path"] == before["model_path"])
        check(tag + "no language-ID: offers importing it", "Ready for Hebrew" in st and "import ggml-tiny-q5_1.bin" in st and pg.is_visible("#he_import"), st)
        pick(pg, "D:\\usb\\ggml-tiny-q5_1.bin")
        pg.click("#he_import")
        pg.wait_for_timeout(100)
        pg.click("#imp_file")
        pg.wait_for_timeout(900)
        st = pg.inner_text("#he_status")
        check(tag + "language-ID imported: complete", "Ready for Hebrew" in st and "language-ID" not in st and not pg.is_visible("#he_import"), st)
        pg.evaluate("window.__goPage('settings-cleanup')")
        pg.wait_for_timeout(300)
        st = pg.inner_text("#llm_dl_status")
        check(tag + "cleanup: Import only, says how", pg.is_visible("#llm_dl_import") and not pg.is_visible("#llm_dl_start") and "then Import from file" in st, st)
        check(tag + "never model_download_start", calls(pg, "model_download_start") == [])
        pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
