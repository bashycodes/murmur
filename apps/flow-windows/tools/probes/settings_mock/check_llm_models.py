"""Headless check of the AI cleanup model downloads (0.11.0) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_llm_models.py [port]
Settings -> AI cleanup -> On this PC: the "Download a model" row lists the
pinned GGUFs (two Qwen, Llama 3.2 3B; not the speech models; Speech to text's
list has no GGUF). It opens on the model recommended for this PC: Qwen2.5 1.5B
without a GPU, Llama 3.2 3B when `llm_backend` reports one.
Download -> confirm dialog -> progress -> done turns local cleanup on with the
file (prefs llm_enabled + llm_model_path) and leaves the speech model alone;
Cancel stops a download; a second model can be downloaded and switched to with
Use; Delete (confirmed) removes it and, when it was in use, turns local cleanup
off. Writes shots/llm-*.png (gitignored).
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []
Q05 = "qwen2.5-0.5b-instruct-q4_k_m"
Q15 = "qwen2.5-1.5b-instruct-q4_k_m"
L3B = "llama-3.2-3b-instruct-q4_k_m"
GPU = {"gpu": True, "backend": "Vulkan", "device": "Intel(R) Arc(TM) 140V GPU", "plugin": "C:\\Program Files\\Murmur\\flow_llm_vulkan.dll"}


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def status(pg):
    return pg.inner_text("#llm_dl_status")


def visible(pg, *ids):
    return [i for i in ids if pg.is_visible("#" + i)]


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        # Start with cleanup off so the download visibly turns it on.
        pg.evaluate("(async () => { const t = window.__TAURI__.core; const p = await t.invoke('get_prefs'); p.llm_enabled = false; await t.invoke('set_prefs', { prefs: p }); })()")
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)
        stt = pg.eval_on_selector_all("#dl_model option", "os => os.map(o => o.value)")
        check(tag + "speech list has no GGUF", stt and not any("qwen" in v for v in stt), str(stt))
        pg.evaluate("window.__goPage('settings-cleanup')")
        pg.wait_for_timeout(300)
        check(tag + "row shown in AI cleanup", pg.is_visible("#llm_dl_row"))
        opts = pg.eval_on_selector_all("#llm_dl_model option", "os => os.map(o => [o.value, o.textContent])")
        check(tag + "lists the three cleanup models", [o[0] for o in opts] == [Q05, Q15, L3B], str(opts))
        check(tag + "sizes in the list", "491 MB" in opts[0][1] and "1117 MB" in opts[1][1] and "2019 MB" in opts[2][1], str(opts))
        check(tag + "no GPU: 1.5B recommended and selected", pg.input_value("#llm_dl_model") == Q15 and "recommended" in opts[1][1] and "recommended" not in opts[0][1] + opts[2][1], str(opts))
        hw = pg.inner_text("#llm_dl_hw")
        check(tag + "says it runs on the CPU", "runs on the CPU" in hw and "Qwen2.5 1.5B" in hw, hw)
        pg.select_option("#llm_dl_model", Q05)
        pg.wait_for_timeout(100)
        check(tag + "a picked model stays picked", pg.input_value("#llm_dl_model") == Q05)
        check(tag + "not downloaded: Download only", visible(pg, "llm_dl_start", "llm_dl_use", "llm_dl_delete", "llm_dl_stop") == ["llm_dl_start"] and pg.inner_text("#llm_dl_start") == "Download 491 MB…")
        check(tag + "status says not downloaded", "Not downloaded" in status(pg), status(pg))
        if scheme == "light":
            pg.locator("#llm_dl_row").screenshot(path=str(out / "llm-row-missing.png"))
        before = prefs(pg)
        # Cancel a running download.
        pg.evaluate("window.__dlSteps = 40; window.__dlTickMs = 40")
        pg.click("#llm_dl_start")
        pg.wait_for_timeout(100)
        check(tag + "confirm dialog first", pg.is_visible("#dl_confirm") and "Download Qwen2.5 0.5B Instruct?" in pg.inner_text("#dl_title"))
        check(tag + "confirm shows the pinned URL and the GGUF path",
              "huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/9217f5db79a29953eb74d5343926648285ec7e67/" in pg.inner_text("#dl_from")
              and pg.inner_text("#dl_dest").endswith("qwen2.5-0.5b-instruct-q4_k_m.gguf"), pg.inner_text("#dl_from"))
        pg.click("#dl_ok")
        pg.wait_for_timeout(250)
        check(tag + "progress + Cancel while downloading", pg.is_visible("#llm_dl_bar") and pg.is_visible("#llm_dl_stop") and not pg.is_visible("#llm_dl_start") and "of 491 MB" in status(pg), status(pg))
        if scheme == "light":
            pg.locator("#llm_dl_row").screenshot(path=str(out / "llm-row-downloading.png"))
        pg.click("#llm_dl_stop")
        pg.wait_for_timeout(300)
        check(tag + "cancelled", "cancelled" in status(pg) and pg.is_visible("#llm_dl_start") and not prefs(pg)["llm_enabled"], status(pg))
        # Download for real.
        pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
        pg.click("#llm_dl_start")
        pg.wait_for_timeout(100)
        pg.click("#dl_ok")
        pg.wait_for_timeout(900)
        after = prefs(pg)
        check(tag + "done -> local cleanup on with the GGUF",
              after["llm_enabled"] and after["llm_model_path"].endswith("qwen2.5-0.5b-instruct-q4_k_m.gguf"), str((after["llm_enabled"], after["llm_model_path"])))
        check(tag + "speech model untouched", (after["asr_engine"], after["model_path"]) == (before["asr_engine"], before["model_path"]))
        check(tag + "model file field shows it", pg.input_value("#llm_model_path") == after["llm_model_path"])
        check(tag + "This PC switch on", pg.is_checked("#llm_enabled"))
        check(tag + "installed + in use: Delete, no Download/Use",
              visible(pg, "llm_dl_start", "llm_dl_use", "llm_dl_delete", "llm_dl_stop") == ["llm_dl_delete"] and "in use" in status(pg), status(pg))
        check(tag + "list marks it in use", "in use" in pg.eval_on_selector("#llm_dl_model option[value='%s']" % Q05, "o => o.textContent"))
        if scheme == "light":
            pg.locator("#llm_dl_row").screenshot(path=str(out / "llm-row-in-use.png"))
        # The 1.5B: download, then switch back with Use.
        pg.select_option("#llm_dl_model", Q15)
        pg.wait_for_timeout(100)
        check(tag + "1.5B offers its download", pg.inner_text("#llm_dl_start") == "Download 1117 MB…")
        pg.click("#llm_dl_start")
        pg.wait_for_timeout(100)
        pg.click("#dl_ok")
        pg.wait_for_timeout(900)
        check(tag + "1.5B now in use", prefs(pg)["llm_model_path"].endswith("qwen2.5-1.5b-instruct-q4_k_m.gguf"))
        pg.select_option("#llm_dl_model", Q05)
        pg.wait_for_timeout(100)
        check(tag + "0.5B downloaded, not in use: Use + Delete", visible(pg, "llm_dl_start", "llm_dl_use", "llm_dl_delete", "llm_dl_stop") == ["llm_dl_use", "llm_dl_delete"], status(pg))
        pg.click("#llm_dl_use")
        pg.wait_for_timeout(300)
        check(tag + "Use switches to it", prefs(pg)["llm_model_path"].endswith("qwen2.5-0.5b-instruct-q4_k_m.gguf") and "in use" in status(pg), status(pg))
        # Delete the one not in use: cleanup stays on.
        pg.select_option("#llm_dl_model", Q15)
        pg.wait_for_timeout(100)
        pg.click("#llm_dl_delete")
        pg.wait_for_timeout(100)
        check(tag + "delete asks first", pg.is_visible("#llm_del_confirm") and "Delete Qwen2.5 1.5B Instruct?" in pg.inner_text("#llm_del_title"))
        pg.click("#llm_del_cancel")
        pg.wait_for_timeout(100)
        check(tag + "cancel keeps it", pg.is_visible("#llm_dl_delete"))
        pg.click("#llm_dl_delete")
        pg.wait_for_timeout(100)
        pg.click("#llm_del_ok")
        pg.wait_for_timeout(300)
        check(tag + "deleted, cleanup stays on", "Deleted" in status(pg) and pg.is_visible("#llm_dl_start") and prefs(pg)["llm_enabled"], status(pg))
        # Delete the one in use: cleanup turns off.
        pg.select_option("#llm_dl_model", Q05)
        pg.wait_for_timeout(100)
        pg.click("#llm_dl_delete")
        pg.wait_for_timeout(100)
        check(tag + "in-use delete warns", "turns off" in pg.inner_text("#llm_del_body"))
        pg.click("#llm_del_ok")
        pg.wait_for_timeout(300)
        p2 = prefs(pg)
        check(tag + "in-use delete turns local cleanup off", not p2["llm_enabled"] and p2["llm_model_path"] is None and not pg.is_checked("#llm_enabled"), str((p2["llm_enabled"], p2["llm_model_path"])))
        check(tag + "deleted -> Download again", pg.is_visible("#llm_dl_start") and not pg.is_visible("#llm_dl_delete"))
        pg.close()
    # A PC whose cleanup runs on a GPU: Llama 3.2 3B is the pick.
    for scheme in ["light", "dark"]:
        tag = f"[gpu {scheme}] "
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.add_init_script("window.__llmBackend = %s" % __import__("json").dumps(GPU))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        pg.evaluate("window.__goPage('settings-cleanup')")
        pg.wait_for_timeout(300)
        opts = pg.eval_on_selector_all("#llm_dl_model option", "os => os.map(o => [o.value, o.textContent])")
        check(tag + "GPU: Llama 3.2 3B recommended and selected", pg.input_value("#llm_dl_model") == L3B and "recommended" in opts[2][1] and "recommended" not in opts[1][1], str(opts))
        hw = pg.inner_text("#llm_dl_hw")
        check(tag + "names the graphics chip", "graphics chip" in hw and "Arc(TM) 140V" in hw and "Llama 3.2 3B" in hw, hw)
        check(tag + "Llama licence credit", "Built with Llama" in status(pg) and pg.inner_text("#llm_dl_start") == "Download 2019 MB…", status(pg))
        if scheme == "light":
            pg.locator("#llm_dl_row").screenshot(path=str(out / "llm-row-gpu.png"))
        pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
        pg.click("#llm_dl_start")
        pg.wait_for_timeout(100)
        check(tag + "confirm shows the pinned Llama URL", "huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/" in pg.inner_text("#dl_from"), pg.inner_text("#dl_from"))
        pg.click("#dl_ok")
        pg.wait_for_timeout(1000)
        check(tag + "Llama in use", prefs(pg)["llm_model_path"].endswith("Llama-3.2-3B-Instruct-Q4_K_M.gguf") and "in use" in status(pg), status(pg))
        desc = pg.inner_text("#aieng_desc_local")
        check(tag + "ready line says it runs on the graphics chip", "graphics chip (Intel(R) Arc(TM) 140V GPU)" in desc, desc)
        pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
