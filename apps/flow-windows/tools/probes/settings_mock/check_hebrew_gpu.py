"""Headless check of Hebrew whisper on the GPU (D42) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_hebrew_gpu.py [port]
Speech to text -> Hebrew model row. On a PC without a GPU for whisper
(`whisper_backend` says CPU) Zipformer stays "(recommended)" and choosing
ivrit.ai Whisper turbo warns that it would run on the CPU, slowly. On a PC
with a GPU (window.__whisperBackend) ivrit.ai Whisper turbo is the
"(recommended)" one; its download (574 MB + the language-ID model, Apache-2.0
credit) makes it the Hebrew route, and the ready line says where Hebrew runs
(the graphics chip, its name and Vulkan). Zipformer says it runs on the CPU.
Writes shots/he-gpu-*.png (gitignored).
"""
import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []
GPU = {"gpu": True, "backend": "Vulkan", "device": "Intel(R) Arc(TM) 140V GPU (16GB)",
       "plugin": "C:\\Program Files\\Murmur\\flow_whisper_vulkan.dll", "error": None}


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def options(pg):
    return pg.eval_on_selector_all("#hebrew_model option", "os => os.map(o => [o.value, o.textContent])")


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        # A PC without a GPU for whisper (the mock's default).
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)
        o = options(pg)
        check(tag + "[cpu] ivrit turbo offered, Zipformer recommended",
              ["zipformer", "Zipformer (recommended)"] in o and ["ivrit_turbo", "ivrit.ai Whisper turbo"] in o, str(o))
        pg.select_option("#hebrew_model", "ivrit_turbo")
        pg.wait_for_timeout(300)
        check(tag + "[cpu] ivrit turbo saved", prefs(pg)["hebrew_model"] == "ivrit_turbo")
        st = pg.inner_text("#he_status")
        check(tag + "[cpu] warns it would run on the CPU", "Not downloaded" in st and "run on the CPU" in st and "Recommended here: Zipformer Hebrew" in st, st)
        check(tag + "[cpu] download button with size", pg.inner_text("#he_download") == "Download 606 MB…", pg.inner_text("#he_download"))
        credit = pg.inner_text("#he_credit")
        check(tag + "[cpu] Apache-2.0 credit", "Apache-2.0" in credit and "ivrit.ai" in credit and pg.is_visible("#he_credit [data-open-url*='tagidu']"), credit)
        if scheme == "light":
            pg.locator("#he_row").screenshot(path=str(out / "he-gpu-cpu-pc.png"))
        pg.close()

        # A PC with a Vulkan GPU.
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.add_init_script("window.__whisperBackend = %s" % json.dumps(GPU))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)
        o = options(pg)
        check(tag + "[gpu] ivrit turbo recommended", ["ivrit_turbo", "ivrit.ai Whisper turbo (recommended)"] in o and ["zipformer", "Zipformer"] in o, str(o))
        st = pg.inner_text("#he_status")
        check(tag + "[gpu] Zipformer (still chosen) says nothing about the GPU", "graphics chip" not in st, st)
        pg.select_option("#hebrew_model", "ivrit_turbo")
        pg.wait_for_timeout(300)
        st = pg.inner_text("#he_status")
        check(tag + "[gpu] before download: says this PC runs it on the GPU", "Not downloaded" in st and "graphics chip (Intel(R) Arc(TM) 140V GPU (16GB), Vulkan)" in st, st)
        before = prefs(pg)
        pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
        pg.click("#he_download")
        pg.wait_for_timeout(100)
        check(tag + "[gpu] confirm dialog", pg.is_visible("#dl_confirm") and "Download ivrit.ai Whisper turbo?" in pg.inner_text("#dl_title"), pg.inner_text("#dl_title"))
        check(tag + "[gpu] confirm: model + language-ID file, from the Tagidu repo",
              "606 MB in 2 files" in pg.inner_text("#dl_size")
              and "huggingface.co/tagidu/ivrit-whisper-large-v3-turbo-ggml-quantized" in pg.inner_text("#dl_from")
              and pg.inner_text("#dl_dest").endswith("ggml-ivrit-large-v3-turbo-q5_0.bin"),
              pg.inner_text("#dl_size") + " | " + pg.inner_text("#dl_from"))
        pg.click("#dl_ok")
        pg.wait_for_timeout(800)
        after = prefs(pg)
        check(tag + "[gpu] done -> Hebrew route, default untouched",
              after["hebrew_model"] == "ivrit_turbo" and after["model_path"] == before["model_path"] and after["asr_engine"] == before["asr_engine"], str(after["hebrew_model"]))
        st = pg.inner_text("#he_status")
        check(tag + "[gpu] ready line says where Hebrew runs",
              "Ready for Hebrew" in st and "Runs on the graphics chip (Intel(R) Arc(TM) 140V GPU (16GB), Vulkan)" in st and not pg.is_visible("#he_download"), st)
        if scheme == "light":
            pg.locator("#he_row").screenshot(path=str(out / "he-gpu-ready.png"))
        pg.close()

    # Zipformer ready: runs on the CPU.
    pg = b.new_page(viewport={"width": 1280, "height": 900})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html?he=nolid")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__goPage('settings-speech')")
    pg.wait_for_timeout(300)
    st = pg.inner_text("#he_status")
    check("[zipformer] ready on the CPU", "Ready for Hebrew" in st and "Runs on the CPU." in st, st)
    pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
