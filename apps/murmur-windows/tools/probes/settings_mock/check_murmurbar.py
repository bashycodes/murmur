"""Headless check of the Murmur bar overlay (overlay.html) against mock-overlay.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_murmurbar.py [port] [shot-dir]
Drives each artboard state (docs/design/app-design/MurmurBar.dc.html) in light
and dark, checks the hands-free pill geometry that src/overlay.rs hit-tests
(187 x 36, bottom 14 px), that ✕ / ■ send overlay_action and that the
error / fell-back bubbles' "Fix →" / "Choose a model →" open their page,
that the link's box is reported to Rust (overlay_action link_rect; null once
the bubble goes) and that the `theme` pref (overlay_theme / overlay:theme)
overrides the system scheme. With shot-dir,
writes <state>-<scheme>.png per state (not committed). Exits non-zero on
the first failed check.
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "8765"
shots = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else None
url = f"http://127.0.0.1:{port}/overlay.html"


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        sys.exit(1)


STATES = {
    "idle": [("dictation:status", {"state": "resting"})],
    "hold": [("dictation:status", {"state": "active_ptt"}), ("audio:level", 0.15)],
    "hold-preview": [
        ("dictation:status", {"state": "active_ptt"}),
        ("audio:level", 0.12),
        ("dictation:live-route", {"text": "Transcribing via this PC…", "engine": "local"}),
        ("dictation:preview", "so the plan for tomorrow is to ship the murmur bar"),
    ],
    "handsfree": [("dictation:status", {"state": "active_locked"}), ("audio:level", 0.2)],
    "handsfree-maxwarn": [
        ("dictation:status", {"state": "active_locked"}),
        ("audio:level", 0.2),
        ("dictation:max-warning", 30000),
    ],
    "command": [("dictation:status", {"state": "active_command"}), ("audio:level", 0.15)],
    "working": [("dictation:status", {"state": "processing"})],
    "done-L": [("dictation:route", {"letter": "L", "fallback": False}), ("dictation:status", {"state": "resting"})],
    "done-S": [("dictation:route", {"letter": "S", "fallback": False}), ("dictation:status", {"state": "resting"})],
    "done-C": [("dictation:route", {"letter": "C", "fallback": False}), ("dictation:status", {"state": "resting"})],
    "fellback": [
        ("dictation:status", {"state": "processing"}),
        ("dictation:route", {"letter": "L", "fallback": True, "message": "OpenRouter rejected the key. Used this PC instead."}),
        ("dictation:status", {"state": "resting"}),
    ],
    "error": [("dictation:status", {"state": "error", "message": "Speech model not found: ggml-base.bin"})],
    "notice": [("dictation:status", {"state": "notice", "message": "No speech heard"})],
    "error-server": [("dictation:status", {"state": "error", "message": "Transcription failed: server unreachable"})],
    "error-cleanup": [("dictation:status", {"state": "error", "message": "AI cleanup model not found: model.gguf"})],
}

# Bubble link -> settings page it opens (overlay_action open_settings).
FIX = {
    "fellback": ("Fix →", "settings-openrouter"),
    "error": ("Choose a model →", "settings-speech"),
    "error-server": ("Fix →", "settings-server"),
    "error-cleanup": ("Choose a model →", "settings-cleanup"),
}

def actions(pg):
    return pg.evaluate("window.__actions")


def clicks(pg):
    """overlay_action calls other than link-box reports."""
    return [a for a in actions(pg) if a["cmd"] == "overlay_action" and a["args"]["action"] != "link_rect"]


def link_rects(pg):
    return [a["args"]["rect"] for a in actions(pg) if a["cmd"] == "overlay_action" and a["args"]["action"] == "link_rect"]


with sync_playwright() as p:
    b = p.chromium.launch()
    for scheme in ("light", "dark"):
        ctx = b.new_context(viewport={"width": 512, "height": 614}, color_scheme=scheme)
        for name, events in STATES.items():
            pg = ctx.new_page()
            errors = []
            pg.on("pageerror", lambda e: errors.append(str(e)))
            pg.goto(url)
            pg.add_style_tag(content="html{background:%s}" % ("#f5f4f0" if scheme == "light" else "#202020"))
            for ev, payload in events:
                pg.evaluate("([e, p]) => window.__emit(e, p)", [ev, payload])
            pg.wait_for_timeout(450)
            check(not errors, f"{scheme} {name}: no page errors {errors}")
            if name == "handsfree":
                box = pg.locator("#pill").bounding_box()
                check(abs(box["width"] - 187) < 0.5 and abs(box["height"] - 36) < 0.5, f"{scheme} hands-free pill 187x36 ({box})")
                check(abs(614 - (box["y"] + box["height"]) - 14) < 0.5, f"{scheme} hands-free pill bottom gap 14")
                for bid in ("btn-cancel", "btn-finish"):
                    bb = pg.locator(f"#{bid}").bounding_box()
                    check(bb["x"] >= box["x"] and bb["x"] + bb["width"] <= box["x"] + box["width"], f"{scheme} #{bid} inside pill")
                pg.locator("#btn-cancel").dispatch_event("mousedown")
                pg.locator("#btn-finish").dispatch_event("mousedown")
                acts = clicks(pg)
                check([a["args"]["action"] for a in acts] == ["cancel", "finish"], f"{scheme} overlay_action cancel/finish")
                check(link_rects(pg) == [], f"{scheme} handsfree: no link box claimed")
            if name.startswith("done-") or name == "fellback":
                check(pg.locator("#pill[data-done] #engine-badge").is_visible(), f"{scheme} {name}: done pill shows letter")
            if name == "fellback":
                check("rejected the key" in pg.locator("#preview").inner_text(), f"{scheme} fell-back notice bubble")
            if name in FIX:
                label, page = FIX[name]
                fix = pg.locator("#preview-fix")
                check(fix.is_visible() and fix.inner_text() == label, f"{scheme} {name}: link {label!r}")
                bb = fix.bounding_box()
                rects = link_rects(pg)
                got = rects[-1] if rects else None
                want = {"left": bb["x"], "top": bb["y"], "right": bb["x"] + bb["width"], "bottom": bb["y"] + bb["height"]}
                check(
                    got is not None and all(abs(got[k] - want[k]) < 0.5 for k in want),
                    f"{scheme} {name}: link box reported {got} ~ {want}",
                )
                check(want["right"] - want["left"] <= 200 and want["bottom"] - want["top"] <= 40, f"{scheme} {name}: link box within overlay.rs MAX_LINK_*")
                fix.dispatch_event("mousedown")
                acts = clicks(pg)
                check(acts[-1:] == [{"cmd": "overlay_action", "args": {"action": "open_settings", "page": page}}], f"{scheme} {name}: opens {page}")
                # Bubble goes (next dictation / resting) -> click-through again.
                pg.evaluate("([e, p]) => window.__emit(e, p)", ["dictation:status", {"state": "active_ptt"}])
                pg.wait_for_timeout(100)
                check(link_rects(pg)[-1:] == [None], f"{scheme} {name}: link box cleared when the bubble hides")
            if name in ("notice", "hold-preview"):
                check(not pg.locator("#preview-fix").is_visible(), f"{scheme} {name}: no link")
                check(link_rects(pg) == [], f"{scheme} {name}: no link box claimed")
            if name == "idle":
                dark = pg.evaluate("document.documentElement.classList.contains('lw-dark')")
                check(dark == (scheme == "dark"), f"{scheme} theme=system follows the OS")
                other = "light" if scheme == "dark" else "dark"
                pg.evaluate("([e, p]) => window.__emit(e, p)", ["overlay:theme", other])
                dark = pg.evaluate("document.documentElement.classList.contains('lw-dark')")
                check(dark == (other == "dark"), f"{scheme} overlay:theme {other} overrides the OS")
                pg2 = ctx.new_page()
                pg2.add_init_script("window.__theme = '%s'" % other)
                pg2.goto(url)
                pg2.wait_for_timeout(100)
                dark = pg2.evaluate("document.documentElement.classList.contains('lw-dark')")
                check(dark == (other == "dark"), f"{scheme} overlay_theme {other} at load overrides the OS")
                pg2.close()
            if name == "handsfree-maxwarn":
                check("stops in 30" in pg.locator("#max-warn").inner_text(), f"{scheme} max-warning countdown")
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                pg.screenshot(path=str(shots / f"{name}-{scheme}.png"), clip={"x": 0, "y": 434, "width": 512, "height": 180})
            pg.close()
        ctx.close()
    b.close()
print("all ok")
