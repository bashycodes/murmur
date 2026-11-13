"""Headless check of setup's "Download from" (D79) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_download_source.py [port]
An air-gapped install that picks This PC on step 2: step 3 has no switch
(it lives in Settings → Speech to text, UX review on 1.3.0) and downloads
from Hugging Face. Step 4's "On this PC" cleanup offers "Download from:
Your server | Hugging Face"; Your server with none saved shows step 2's
server fields there (Connect saves the address only: speech stays on This
PC) and hides Download and the Hugging Face link until it is saved; then
the cleanup model downloads from the server. Back on step 3 a speech model
the server lacks says so instead of offering Download. Back on step 2 the
server fields are where they were. Writes shots/src-*.png (gitignored).
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


def step(pg):
    return pg.evaluate("[...document.querySelectorAll('#onboarding [data-ob-step]')].filter(p => !p.hidden).map(p => p.dataset.obStep).join(',')")


def nxt(pg):
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
    pg.wait_for_timeout(300)


def on(pg, s, src):
    return pg.get_attribute(f"[data-ob-step={s}] [data-ob-src={src}]", "aria-checked") == "true"


def shown(pg, sel):
    return pg.is_visible(sel)


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1280, "height": 860})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html?onboard=1")
    pg.wait_for_timeout(600)
    nxt(pg)
    pg.click("[data-ob-engine=pc]")
    nxt(pg)
    check("step 3 after This PC", step(pg) == "model", step(pg))

    # Step 3: no "Download from" (Settings → Speech to text has it); Hugging Face.
    check("no switch on step 3", not pg.query_selector("[data-ob-step=model] [data-ob-src-box]"))
    check("no server fields on step 3", not shown(pg, "[data-ob-step=model] #ob_server_detail"))
    pg.evaluate("window.__dlSteps = 2; window.__dlTickMs = 30")
    pg.click("[data-ob-dl=parakeet]")
    pg.wait_for_timeout(150)
    check("speech model from Hugging Face", "huggingface.co" in pg.inner_text("#dl_from"), pg.inner_text("#dl_from"))
    pg.click("#dl_ok")
    pg.wait_for_timeout(800)
    check("speech model installed", pg.inner_text("[data-ob-model=parakeet] [data-ob-chip]") == "Installed")
    pg.screenshot(path=str(out / "src-1-step3.png"))

    # Step 4: cleanup on this PC, the switch, on Hugging Face without a server.
    nxt(pg)
    check("step 4", step(pg) == "cleanup", step(pg))
    pg.click("[data-ob-cleanup=pc]")
    pg.wait_for_timeout(200)
    check("switch shown", shown(pg, "[data-ob-step=cleanup] [data-ob-src-box]"))
    check("both sources offered", shown(pg, "[data-ob-step=cleanup] [data-ob-src=server]") and shown(pg, "[data-ob-step=cleanup] [data-ob-src=huggingface]"))
    check("on Hugging Face without a server", on(pg, "cleanup", "huggingface"))
    check("HF link shown", shown(pg, "#ob_llm_hf"))
    check("no server fields yet", not shown(pg, "#ob_server_detail"))

    # Your server with none saved: step 2's fields, here.
    pg.click("[data-ob-step=cleanup] [data-ob-src=server]")
    pg.wait_for_timeout(300)
    check("server picked", on(pg, "cleanup", "server"))
    check("server fields on step 4", shown(pg, "[data-ob-step=cleanup] #ob_server_detail"))
    check("button says Connect", pg.inner_text("#ob_test_conn") == "Connect")
    check("Download waits for the server", not shown(pg, "#ob_llm_dl_start"))
    check("HF link hidden", not shown(pg, "#ob_llm_hf"))
    check("hint asks for the server", "Add your server" in pg.inner_text("[data-ob-step=cleanup] [data-ob-src-hint]"))
    pg.screenshot(path=str(out / "src-2-server-form.png"))
    pg.click("#ob_test_conn")
    pg.wait_for_timeout(200)
    check("Connect needs an address", "address" in pg.inner_text("#ob_host_err"))
    pg.fill("#ob_host", "100.64.0.10")
    pg.fill("#ob_token", "abcdefghijklmnop1234")
    pg.click("#ob_test_conn")
    pg.wait_for_timeout(600)
    d = pg.evaluate("window.__TAURI__.core.invoke('dev_get')")["form"]
    check("address saved", d["host"] == "100.64.0.10", str(d))
    check("speech stays on This PC", d["route_order"][0] == "client", str(d["route_order"]))
    prefs = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")
    check("model_source saved", prefs["model_source"] == "server")
    check("fields gone once saved", not shown(pg, "#ob_server_detail"))
    check("hint names the server", "100.64.0.10" in pg.inner_text("[data-ob-step=cleanup] [data-ob-src-hint]"))
    check("step 4 switch on the server", on(pg, "cleanup", "server"))
    check("cleanup model row", shown(pg, "#ob_llm_dl") and "Qwen2.5 1.5B" in pg.inner_text("#ob_llm_dl_title"))
    check("cleanup Download from the server", shown(pg, "#ob_llm_dl_start"))
    check("cleanup HF link hidden", not shown(pg, "#ob_llm_hf"))
    pg.screenshot(path=str(out / "src-4-cleanup-server.png"))
    pg.click("#ob_llm_dl_start")
    pg.wait_for_timeout(150)
    check("cleanup confirm says your server", "Your server" in pg.inner_text("#dl_from"))
    pg.click("#dl_ok")
    pg.wait_for_timeout(1000)
    prefs = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")
    check("cleanup model downloaded and set", (prefs.get("llm_model_path") or "").endswith("qwen2.5-1.5b-instruct-q4_k_m.gguf") and "downloaded" in pg.inner_text("#ob_llm_dl_desc"), str(prefs.get("llm_model_path")))

    # Back on step 3, downloads now come from the server: Whisper base is
    # there, a model it lacks says so (and where to change it) instead of Download.
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=back]")
    pg.wait_for_timeout(400)
    pg.click("#ob_model_more")
    pg.wait_for_timeout(100)
    check("whisper base from the server", shown(pg, "[data-ob-dl=whisper]"))
    pg.screenshot(path=str(out / "src-3-step3-server.png"))
    pg.click("[data-ob-dl=whisper]")
    pg.wait_for_timeout(150)
    check("speech confirm says your server", "Your server, 100.64.0.10" in pg.inner_text("#dl_from"), pg.inner_text("#dl_from"))
    pg.click("#dl_cancel")
    pg.wait_for_timeout(100)
    check("Continue waits for a picked model that is not there", pg.is_disabled("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]"))
    pg.click("[data-ob-model=parakeet]")
    pg.wait_for_timeout(100)
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=next]")
    pg.wait_for_timeout(300)

    # Step 4 again: back to Hugging Face.
    pg.click("[data-ob-step=cleanup] [data-ob-src=huggingface]")
    pg.wait_for_timeout(400)
    prefs = pg.evaluate("window.__TAURI__.core.invoke('get_prefs')")
    check("Hugging Face picked", on(pg, "cleanup", "huggingface") and prefs["model_source"] == "huggingface")
    check("cleanup HF link back", shown(pg, "#ob_llm_hf"))
    check("no server fields with a server saved", not shown(pg, "#ob_server_detail"))
    pg.screenshot(path=str(out / "src-5-cleanup-hf.png"))

    # Back: step 2 has its fields again.
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=back]")
    pg.wait_for_timeout(200)
    pg.click("#onboarding [data-ob-step]:not([hidden]) [data-ob=back]")
    pg.wait_for_timeout(200)
    pg.click("[data-ob-engine=server]")
    pg.wait_for_timeout(100)
    check("server fields back on step 2", shown(pg, "[data-ob-step=engine] #ob_server_detail") and pg.inner_text("#ob_test_conn") == "Test")
    pg.screenshot(path=str(out / "src-6-engine.png"))
    check("no page errors", not errors, "; ".join(errors[:5]))
    b.close()

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
