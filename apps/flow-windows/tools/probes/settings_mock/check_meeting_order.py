"""Headless check of the D47 meeting transcription order against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_meeting_order.py [port]
Settings -> Speech to text -> Meetings: three rows (this PC, your server,
OpenRouter) with the same ranked-list widget as Engines and Cleanup order
(rank, up/down, switch, badge). Unedited it shows dictation's order and
switches and says so; moving a row or flipping a switch saves the whole list
(dev_meeting_routes_set) without touching dictation's order; an off row
keeps its place; OpenRouter needs the speech consent first. Also re-checks
the Engines list (shared widget code). Writes shots/meeting-order-*.png
(gitignored).
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


def rows(pg, list_id, attr):
    """[(key, rank, is-off)] of the shown rows in DOM order."""
    return pg.eval_on_selector_all(
        f"#{list_id} [data-{attr}]",
        "rs => rs.filter(r => !r.hidden).map(r => [r.dataset."
        + attr
        + ", r.querySelector('.lw-rank').textContent, r.classList.contains('is-off')])",
    )


def saved(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('dev_get').then(v => [v.meetings, v.form.route_order])")


def click(pg, sel):
    pg.click(sel)
    pg.wait_for_timeout(200)


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(f"http://127.0.0.1:{port}/settings.html")
        pg.wait_for_timeout(500)
        pg.evaluate("window.__goPage('settings-speech')")
        pg.wait_for_timeout(300)

        check(tag + "section is shown and called Meetings",
              pg.is_visible("#meet_group") and "Meetings" in pg.inner_text("#meet_group"))
        r = rows(pg, "meet_routes", "meeting")
        check(tag + "unedited = dictation's order and switches",
              r == [["server", "1", False], ["client", "2", False], ["cloud", "3", True]], str(r))
        check(tag + "says it follows the engines above", "Same as the engines above" in pg.inner_text("#meet_follow"))
        check(tag + "same widget as Engines (rank, tile, up/down, switch, badge)",
              all(pg.query_selector(f"#meet_row_{x} {s}") for x in ["client", "server", "cloud"]
                  for s in [".lw-rank", ".lw-tile", "[data-move=up]", "[data-move=down]", ".fl-toggle", ".fl-badge"]))
        check(tag + "first up / last down disabled",
              pg.is_disabled("#meet_row_server [data-move=up]") and pg.is_disabled("#meet_row_cloud [data-move=down]"))
        check(tag + "server badge reflects health", pg.inner_text("#meet_badge_server") in ("Connected", "On", "Unreachable"),
              pg.inner_text("#meet_badge_server"))
        pg.screenshot(path=str(out / f"meeting-order-default-{scheme}.png"), full_page=True)

        # Move this PC first: the meeting list is saved, dictation's is not.
        click(pg, "#meet_row_client [data-move=up]")
        r = rows(pg, "meet_routes", "meeting")
        check(tag + "this PC moved first", [x[0] for x in r] == ["client", "server", "cloud"], str(r))
        m, dict_order = saved(pg)
        check(tag + "saved: all three, flags kept",
              m["routes"] == [{"route": "client", "enabled": True}, {"route": "server", "enabled": True},
                              {"route": "cloud", "enabled": False}], str(m))
        check(tag + "no longer follows dictation", not m["follows_dictation"] and pg.inner_text("#meet_follow") == "")
        check(tag + "dictation order unchanged", dict_order == ["server", "client", "cloud"], str(dict_order))
        e = rows(pg, "dev_routes", "route")
        check(tag + "Engines list unchanged", [x[0] for x in e] == ["server", "client", "cloud"], str(e))

        # Server off: keeps its place, dimmed, badge Off; dictation's server stays on.
        click(pg, "#meet_row_server .fl-toggle")
        r = rows(pg, "meet_routes", "meeting")
        check(tag + "server off keeps its place", r[1] == ["server", "2", True], str(r))
        check(tag + "server badge Off", pg.inner_text("#meet_badge_server") == "Off")
        check(tag + "dictation server switch still on", pg.is_checked("#dev_allow_server"))
        m, _ = saved(pg)
        check(tag + "saved server off", {"route": "server", "enabled": False} in m["routes"], str(m))

        # OpenRouter without the speech consent: refused, stays off.
        click(pg, "#meet_row_cloud .fl-toggle")
        check(tag + "OpenRouter needs the consent", "Accept sending audio" in pg.inner_text("#meet_order_msg"),
              pg.inner_text("#meet_order_msg"))
        check(tag + "OpenRouter stays off", not pg.is_checked("#meet_on_cloud"))
        # With the consent given (Speech to text OpenRouter switch, then off again).
        pg.evaluate("""window.__TAURI__.core.invoke('dev_or_set_stt', {form: {allow: false, consent: true,
                        language: '', send_dictionary: true, stt_path: 'auto'}})""")
        click(pg, "#meet_row_cloud .fl-toggle")
        check(tag + "OpenRouter on after consent", pg.is_checked("#meet_on_cloud") and pg.inner_text("#meet_order_msg") == "")
        click(pg, "#meet_row_cloud [data-move=up]")
        click(pg, "#meet_row_cloud [data-move=up]")
        r = rows(pg, "meet_routes", "meeting")
        check(tag + "OpenRouter first", r[0] == ["cloud", "1", False], str(r))
        pg.screenshot(path=str(out / f"meeting-order-moved-{scheme}.png"), full_page=True)

        # Shared widget code: the Engines list still moves (dictation only).
        click(pg, "#eng_row_client [data-move=up]")
        e = rows(pg, "dev_routes", "route")
        check(tag + "Engines move still works", [x[0] for x in e][:2] == ["client", "server"], str(e))
        m, dict_order = saved(pg)
        check(tag + "and leaves the meeting order alone", m["routes"][0]["route"] == "cloud", str(m))
        pg.close()
    check("no page errors", not errors, "; ".join(errors))
    b.close()

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
