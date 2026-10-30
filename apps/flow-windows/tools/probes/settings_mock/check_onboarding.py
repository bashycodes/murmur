"""Headless walk through first-run setup against mock-tauri.js (?onboard=1).

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_onboarding.py [port]
Opens the app as a fresh install (onboarded false): setup covers the shell.
Walks every step in light and dark: picks each engine (server: address +
token + Test; OpenRouter: key + consent), a model location that is missing,
a download (D31: confirm dialog 3a, cancel, progress 3b, done -> prefs and
model ready), a picked folder that loads, each cleanup choice, then the
practice box (a
dictation result arrives through dev:route), changes the shortcut and
finishes. Asserts what reached dev_set / dev_or_set_* / set_prefs, that
Finish saves onboarded = true and that Settings → General → Run setup
opens it again. Writes shots/ob-*.png (gitignored).
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


def calls(pg, cmd):
    return pg.evaluate("c => window.__mockCalls.filter(x => x[0] === c).map(x => x[1])", cmd)


def prefs(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")


def dev(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('dev_get')")["form"]


def orv(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('dev_or_get')")


def visible_step(pg):
    return pg.evaluate("[...document.querySelectorAll('#onboarding [data-ob-step]')].filter(p => !p.hidden).map(p => p.dataset.obStep).join(',')")


def nxt(pg, wait=250):
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
    pg.wait_for_timeout(wait)


def back(pg):
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=back]")
    pg.wait_for_timeout(150)


def walk(pg, scheme):
    shot = lambda name: pg.screenshot(path=str(out / f"ob-{name}-{scheme}.png"))
    tag = f"[{scheme}] "
    pg.goto(f"http://127.0.0.1:{port}/settings.html?onboard=1")
    pg.wait_for_timeout(600)
    check(tag + "setup opens on a fresh install", pg.is_visible("#onboarding") and visible_step(pg) == "welcome")
    check(tag + "shell hidden behind it", pg.evaluate("getComputedStyle(document.querySelector('.lw-app')).visibility") == "hidden")
    shot("1-welcome")

    nxt(pg)
    check(tag + "engine step", visible_step(pg) == "engine")
    check(tag + "this PC preselected", pg.get_attribute("[data-ob-engine=pc]", "aria-checked") == "true")
    check(tag + "step counter", pg.inner_text("[data-ob-step=engine] .fl-overline").strip().lower() == "step 2 of 5")
    shot("2-engine")

    # Your server: address required, Test, then Continue saves it first in the order.
    pg.click("[data-ob-engine=server]")
    shot("2b-engine-server")
    nxt(pg)
    check(tag + "server needs an address", visible_step(pg) == "engine" and "address" in pg.inner_text("#ob_host_err"))
    pg.click("#ob_server_detail [data-open-url]")
    pg.wait_for_timeout(100)
    check(tag + "set up a server opens the docs", any("github.com/bashycodes/murmur" in c["url"] for c in calls(pg, "open_external")))
    pg.fill("#ob_host", "100.64.0.10")
    pg.fill("#ob_token", "abcdefghijklmnop1234")
    pg.click("#ob_test_conn")
    pg.wait_for_timeout(200)
    check(tag + "test connection on click", "Connected" in pg.inner_text("#ob_server_status"), pg.inner_text("#ob_server_status"))
    nxt(pg)
    d = dev(pg)
    check(tag + "server saved", visible_step(pg) == "model" and d["host"] == "100.64.0.10" and d["allow_server"] and d["route_order"][0] == "server", str(d))
    check(tag + "model optional for server", not pg.is_disabled("[data-ob-step=model] [data-ob=next]") and "optional" in pg.inner_text("#ob_model_title"))
    back(pg)

    # OpenRouter: key + consent before it is turned on.
    pg.click("[data-ob-engine=cloud]")
    shot("2c-engine-cloud")
    nxt(pg)
    check(tag + "cloud needs a key", visible_step(pg) == "engine" and "key" in pg.inner_text("#ob_cloud_status"))
    pg.fill("#ob_stt_key", "sk-or-v1-0123456789abcdef")
    nxt(pg)
    check(tag + "cloud needs consent", visible_step(pg) == "engine" and "Tick" in pg.inner_text("#ob_cloud_status"))
    check(tag + "key saved", any(c.get("key") == "sk-or-v1-0123456789abcdef" for c in calls(pg, "dev_cloud_set_key")))
    pg.check("#ob_stt_consent")
    nxt(pg)
    o = orv(pg)
    check(tag + "cloud speech on with consent", visible_step(pg) == "model" and o["stt"]["allow"] and o["stt"]["consent"], str(o["stt"]))
    check(tag + "cloud first", dev(pg)["route_order"][0] == "cloud", str(dev(pg)["route_order"]))
    back(pg)

    # This PC: client on and first.
    pg.click("[data-ob-engine=pc]")
    nxt(pg)
    d = dev(pg)
    check(tag + "this PC first", d["allow_client"] and d["route_order"][0] == "client", str(d))
    check(tag + "model step", visible_step(pg) == "model")
    check(tag + "continue blocked without a model", pg.is_disabled("[data-ob-step=model] [data-ob=next]"))
    check(tag + "models folder shown", "FlowClone" in pg.inner_text("#ob_models_dir"))
    shot("3-model-missing")
    pg.fill("#ob_whisper_path", "C:\\missing\\ggml-base.bin")
    pg.click("[data-ob-use=whisper]")
    pg.wait_for_timeout(400)
    check(tag + "missing file reported", "Not found" in pg.inner_text("[data-ob-model=whisper] [data-ob-model-badge]") and pg.is_disabled("[data-ob-step=model] [data-ob=next]"))

    # D31 download: confirm dialog first; Cancel there starts nothing.
    check(tag + "download offered", pg.is_visible("[data-ob-dl=parakeet]") and pg.is_visible("[data-ob-dl=whisper]"))
    check(tag + "download-capable lead", "download it for you" in pg.inner_text("#ob_model_lead") and "fingerprint" in pg.inner_text("#ob_model_note"))
    pg.click("[data-ob-dl=parakeet]")
    pg.wait_for_timeout(150)
    check(tag + "confirm dialog (3a)", pg.is_visible("#dl_confirm") and "Download Parakeet TDT 0.6B?" in pg.inner_text("#dl_title"))
    check(tag + "confirm shows source, size, folder", "huggingface.co" in pg.inner_text("#dl_from") and "661 MB in 4 files" in pg.inner_text("#dl_size") and "FlowClone" in pg.inner_text("#dl_dest") and pg.inner_text("#dl_ok") == "Download 661 MB")
    shot("3c-download-confirm")
    pg.click("#dl_cancel")
    pg.wait_for_timeout(150)
    check(tag + "cancel in the dialog starts nothing", not pg.is_visible("#dl_confirm") and not calls(pg, "model_download_start"))
    # Start, then cancel mid-way (slow ticks).
    pg.evaluate("window.__dlSteps = 40; window.__dlTickMs = 60")
    pg.click("[data-ob-dl=parakeet]")
    pg.wait_for_timeout(100)
    pg.click("#dl_ok")
    pg.wait_for_timeout(500)
    check(tag + "download started on confirm", [c["id"] for c in calls(pg, "model_download_start")] == ["parakeet-tdt-0.6b-v2-int8"])
    check(tag + "progress (3b)", pg.is_visible("[data-ob-model=parakeet] [data-ob-dl-bar]") and " of 661 MB" in pg.inner_text("[data-ob-model=parakeet] [data-ob-dl-line]") and "huggingface.co" in pg.inner_text("[data-ob-model=parakeet] [data-ob-dl-line]"), pg.inner_text("[data-ob-model=parakeet] [data-ob-dl-line]"))
    check(tag + "continue allowed while downloading", not pg.is_disabled("[data-ob-step=model] [data-ob=next]") and "background" in pg.inner_text("#ob_model_hint"))
    check(tag + "other download disabled", pg.is_disabled("[data-ob-dl=whisper]"))
    shot("3d-downloading")
    pg.click("[data-ob-dl-cancel=parakeet]")
    pg.wait_for_timeout(300)
    check(tag + "cancelled", "cancelled" in pg.inner_text("[data-ob-model=parakeet] [data-ob-dl-line]") and pg.is_visible("[data-ob-dl=parakeet]") and pg.is_disabled("[data-ob-step=model] [data-ob=next]"))
    # Whisper base to the end: prefs point at it and it loads.
    pg.evaluate("window.__dlSteps = 4; window.__dlTickMs = 40")
    pg.click("[data-ob-dl=whisper]")
    pg.wait_for_timeout(100)
    pg.click("#dl_ok")
    pg.wait_for_timeout(900)
    p = prefs(pg)
    check(tag + "download done -> prefs", p["asr_engine"] == "whisper" and p["model_path"].endswith("ggml-base.bin"), str(p["model_path"]))
    check(tag + "downloaded model ready", "Ready" in pg.inner_text("[data-ob-model=whisper] [data-ob-model-badge]") and not pg.is_visible("[data-ob-dl=whisper]") and "downloaded" in pg.inner_text("[data-ob-model=whisper] [data-ob-model-desc]"))
    check(tag + "continue after download", not pg.is_disabled("[data-ob-step=model] [data-ob=next]"))
    shot("3e-downloaded")
    pg.evaluate("window.__pickResult = null")
    pg.click("[data-ob-pick=parakeet]")
    pg.wait_for_timeout(200)
    check(tag + "cancelled picker changes nothing", prefs(pg)["asr_engine"] == "whisper")
    pg.evaluate("delete window.__pickResult")
    pg.click("[data-ob-pick=parakeet]")
    pg.wait_for_timeout(400)
    check(tag + "folder picker asked", any(c["kind"] == "folder" for c in calls(pg, "pick_path")))
    p = prefs(pg)
    check(tag + "parakeet saved", p["asr_engine"] == "parakeet" and p["parakeet_model_dir"].startswith("D:"), str(p["asr_engine"]))
    check(tag + "model ready", "Ready" in pg.inner_text("[data-ob-model=parakeet] [data-ob-model-badge]") and not pg.is_disabled("[data-ob-step=model] [data-ob=next]"))
    pg.click("#onboarding [data-ob-step=model] [data-open-url*=sherpa-onnx]")
    pg.wait_for_timeout(100)
    check(tag + "download page opens in the browser", any("sherpa-onnx" in c["url"] for c in calls(pg, "open_external")) and "settings.html" in pg.url)
    shot("3b-model-ready")
    nxt(pg)

    check(tag + "cleanup step", visible_step(pg) == "cleanup")
    shot("4-cleanup")
    pg.click("[data-ob-cleanup=pc]")
    check(tag + "local model row", pg.is_visible("#ob_llm_detail") and pg.inner_text("#ob_sample") == "Lunch is on Thursday.")
    shot("4b-cleanup-pc")
    pg.click("[data-ob-cleanup=cloud]")
    check(tag + "saved key not asked again", "saved" in pg.get_attribute("#ob_clean_key", "placeholder"))
    nxt(pg)
    check(tag + "cleanup needs consent", visible_step(pg) == "cleanup" and "Tick" in pg.inner_text("#ob_cleanup_status"))
    pg.check("#ob_clean_consent")
    shot("4c-cleanup-cloud")
    nxt(pg)
    o = orv(pg)
    check(tag + "cloud cleanup on", visible_step(pg) == "try" and o["cleanup"]["allow"] and o["cleanup"]["consent"], str(o["cleanup"]))
    back(pg)
    pg.click("[data-ob-cleanup=pc]")
    pg.click("[data-ob-pick=llm]")
    pg.wait_for_timeout(200)
    check(tag + "gguf picker fills the field", pg.input_value("#ob_llm_path").endswith(".gguf"))
    nxt(pg)
    p = prefs(pg)
    check(tag + "local cleanup saved", p["llm_enabled"] and p["cleanup_level"] != "none" and p["llm_model_path"].endswith(".gguf") and not orv(pg)["cleanup"]["allow"], f"{p['llm_enabled']} {p['cleanup_level']}")
    back(pg)
    pg.click("[data-ob-cleanup=rules]")
    nxt(pg)
    p = prefs(pg)
    check(tag + "rules only saved", not p["llm_enabled"] and p["cleanup_level"] == "none" and p["spoken_cleanup"])

    check(tag + "try step", visible_step(pg) == "try")
    check(tag + "shortcut keys", pg.eval_on_selector_all("#ob_keys kbd", "ks => ks.map(k => k.textContent)") == ["Ctrl", "Win"])
    check(tag + "practice box focused", pg.evaluate("document.activeElement.id") == "ob_try_box")
    pg.fill("#ob_try_box", "Hey, are we still on for Thursday? I'll bring the slides.")
    pg.evaluate("""window.__emit('dev:route', { last: { engine: 'local', asr_ms: 900, total_ms: 1100, status: 'FORMATTED',
      empty_reason: null, notice: null, at_ms: Date.now() }, circuit: { open: false }, health: null, cloud_circuit: null })""")
    pg.wait_for_timeout(100)
    check(tag + "result shown", "Heard on this PC in 0.9 s" in pg.inner_text("#ob_result_text"), pg.inner_text("#ob_result_text"))
    shot("5-try")
    pg.click("#ob_ptt_change")
    pg.fill("#ob_ptt", "Right Alt")
    pg.press("#ob_ptt", "Tab")
    pg.wait_for_timeout(150)
    check(tag + "shortcut saved", prefs(pg)["shortcuts"]["ptt"] == "Right Alt" and "Right Alt" in pg.inner_text("#ob_keys"))
    pg.select_option("#ob_mic", "USB Mic")
    pg.wait_for_timeout(100)
    check(tag + "microphone saved", prefs(pg)["microphone"] == "USB Mic")
    nxt(pg, 400)
    check(tag + "finish closes setup", not pg.is_visible("#onboarding") and prefs(pg)["onboarded"] is True)
    check(tag + "settings pages refilled", pg.input_value("#ptt") == "Right Alt", pg.input_value("#ptt"))

    pg.evaluate("window.__goPage('settings-general')")
    pg.wait_for_timeout(200)
    pg.click("#run_setup")
    pg.wait_for_timeout(300)
    check(tag + "run setup again", pg.is_visible("#onboarding") and visible_step(pg) == "welcome" and pg.is_hidden("#settings"))
    nxt(pg)
    check(tag + "re-run preselects saved engine", pg.get_attribute("[data-ob-engine=pc]", "aria-checked") == "true")
    back(pg)
    pg.click("[data-ob=skip]")
    pg.wait_for_timeout(200)
    check(tag + "set up later closes", not pg.is_visible("#onboarding"))


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ("light", "dark"):
        pg = b.new_page(viewport={"width": 1280, "height": 800}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.on("console", lambda m: m.type == "error" and errors.append(m.text))
        walk(pg, scheme)
        pg.close()
    # ?dlfail=1: a fingerprint mismatch is shown and can be retried.
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html?onboard=1&dlfail=1")
    pg.wait_for_timeout(600)
    nxt(pg)
    nxt(pg)
    pg.evaluate("window.__dlSteps = 2; window.__dlTickMs = 30")
    pg.click("[data-ob-dl=whisper]")
    pg.wait_for_timeout(100)
    pg.click("#dl_ok")
    pg.wait_for_timeout(600)
    line = pg.inner_text("[data-ob-model=whisper] [data-ob-dl-line]")
    check("fingerprint mismatch shown", "fingerprint" in line and pg.inner_text("[data-ob-dl=whisper]") == "Try again" and prefs(pg)["model_path"] is None, line)
    pg.close()
    # Settings -> Speech to text: Browse + Download a model.
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(500)
    pg.evaluate("window.__goPage('settings-speech')")
    pg.wait_for_timeout(200)
    pg.select_option("#asr_engine", "whisper")
    pg.wait_for_timeout(100)
    pg.click("[data-browse=model_path]")
    pg.wait_for_timeout(300)
    check("settings browse saves the model file", prefs(pg)["model_path"] == "D:\\models\\ggml-base.bin", str(prefs(pg)["model_path"]))
    opts = pg.eval_on_selector_all("#dl_model option", "os => os.map(o => o.textContent)")
    check("settings download row lists the catalogue", pg.is_visible("#dl_row") and len(opts) == 3 and any("downloaded" in o for o in opts), str(opts))
    pg.select_option("#dl_model", "whisper-small-q5_1")
    pg.evaluate("window.__dlSteps = 3; window.__dlTickMs = 30")
    pg.click("#dl_start")
    pg.wait_for_timeout(100)
    check("settings download confirms first", pg.is_visible("#dl_confirm") and "Whisper small" in pg.inner_text("#dl_title"))
    pg.screenshot(path=str(out / "ob-settings-download-confirm.png"))
    pg.click("#dl_ok")
    pg.wait_for_timeout(700)
    check("settings download done", "Downloaded" in pg.inner_text("#dl_status") and prefs(pg)["model_path"].endswith("ggml-small-q5_1.bin"), pg.inner_text("#dl_status"))
    pg.screenshot(path=str(out / "ob-settings-speech.png"))
    pg.evaluate("window.__goPage('settings-server')")
    pg.wait_for_timeout(100)
    pg.click("[data-open-url*='server-setup'] >> visible=true")
    pg.wait_for_timeout(100)
    check("server page link opens", any("server-setup" in c["url"] for c in calls(pg, "open_external")))
    pg.close()
    # An upgrader (onboarded true) never sees it.
    pg = b.new_page(viewport={"width": 1280, "height": 800})
    pg.goto(f"http://127.0.0.1:{port}/settings.html")
    pg.wait_for_timeout(500)
    check("no setup when onboarded", not pg.is_visible("#onboarding"))
    pg.close()
    b.close()
    check("no page errors", not errors, "; ".join(errors[:5]))

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
