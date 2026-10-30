"""Headless check of the D36 cleanup order (0.11.0) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_cleanup_order.py [port]
Settings -> AI cleanup -> Cleanup order: three rows (this PC, your server,
OpenRouter) with the speech-to-text order's widget (rank, up/down, switch,
status badge), default this PC -> your server -> OpenRouter (off). Moving a
row saves the whole order (dev_cleanup_set_engines), the server switch keeps
its place when off, OpenRouter is switched with its consent dialog and keeps
its place, the server's badge follows cleanup_status.server_llm, and the old
order select is gone. Writes shots/cleanup-order-*.png (gitignored).
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


def rows(pg):
    """[(route, rank, is-off)] in DOM order."""
    return pg.eval_on_selector_all(
        "#ai_routes [data-cleanup]",
        "rs => rs.map(r => [r.dataset.cleanup, r.querySelector('.lw-rank').textContent, r.classList.contains('is-off')])",
    )


def engines(pg):
    return pg.evaluate("window.__TAURI__.core.invoke('dev_or_get').then(v => v.cleanup.engines)")


def badge(pg, r):
    return pg.inner_text(f"#aieng_badge_{r}")


def move(pg, r, d):
    pg.click(f"#aieng_row_{r} [data-move={d}]")
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
        pg.evaluate("window.__goPage('settings-cleanup')")
        pg.wait_for_timeout(300)

        check(tag + "section is called Cleanup order",
              "Cleanup order" in pg.inner_text("#cleanup_order_group"))
        check(tag + "old order select is gone", pg.query_selector("#or_cleanup_order") is None)
        r = rows(pg)
        check(tag + "default: this PC, your server, OpenRouter (off)",
              r == [["local", "1", False], ["server", "2", False], ["cloud", "3", True]], str(r))
        check(tag + "same widget as speech order (rank, tile, up/down, switch)",
              all(pg.query_selector(f"#aieng_row_{x} {s}") for x in ["local", "server", "cloud"]
                  for s in [".lw-rank", ".lw-tile", "[data-move=up]", "[data-move=down]", ".fl-toggle"]))
        check(tag + "first up / last down disabled",
              pg.is_disabled("#aieng_row_local [data-move=up]") and pg.is_disabled("#aieng_row_cloud [data-move=down]"))
        check(tag + "server badge Ready (server has an LLM)", badge(pg, "server") == "Ready", badge(pg, "server"))
        check(tag + "server desc names the host", "100.64.0.10" in pg.inner_text("#aieng_desc_server"))
        check(tag + "cloud badge Off", badge(pg, "cloud") == "Off", badge(pg, "cloud"))
        line = pg.inner_text("#ai_engine_line")
        check(tag + "now-line lists this PC then your server", line == "This PC, then your server", line)
        pg.screenshot(path=str(out / f"cleanup-order-default-{scheme}.png"), full_page=True)

        # Move the server below OpenRouter: saved as the whole list.
        move(pg, "server", "down")
        check(tag + "server moved down",
              [x[0] for x in rows(pg)] == ["local", "cloud", "server"], str(rows(pg)))
        e = engines(pg)
        check(tag + "saved: all three, flags kept",
              e == [{"route": "local", "enabled": True}, {"route": "cloud", "enabled": False},
                    {"route": "server", "enabled": True}], str(e))
        move(pg, "server", "up")
        move(pg, "server", "up")
        check(tag + "server first", [x[0] for x in rows(pg)] == ["server", "local", "cloud"], str(rows(pg)))

        # Server switch off: keeps its place, row dimmed, badge Off.
        pg.click("#aieng_row_server .fl-toggle")
        pg.wait_for_timeout(200)
        r = rows(pg)
        check(tag + "server off keeps its place", r[0] == ["server", "1", True], str(r))
        check(tag + "server badge Off", badge(pg, "server") == "Off", badge(pg, "server"))
        check(tag + "saved server off",
              [x for x in engines(pg) if x["route"] == "server"] == [{"route": "server", "enabled": False}])
        check(tag + "now-line drops the server", pg.inner_text("#ai_engine_line") == "This PC")

        # Server without an LLM: badge says so.
        pg.evaluate("window.__serverLlm = false")
        pg.click("#aieng_row_server .fl-toggle")
        pg.wait_for_timeout(200)
        check(tag + "server without LLM is flagged", badge(pg, "server") == "No AI model", badge(pg, "server"))
        check(tag + "and skipped in the now-line", pg.inner_text("#ai_engine_line") == "This PC")
        pg.evaluate("window.__serverLlm = true")

        # OpenRouter on: consent dialog, keeps its place (last).
        pg.click("#aieng_row_cloud .fl-toggle")
        pg.wait_for_timeout(200)
        if pg.is_visible("#or_cleanup_consent_ok"):
            pg.click("#or_cleanup_consent_ok")
            pg.wait_for_timeout(300)
        r = rows(pg)
        check(tag + "OpenRouter on keeps its place", r[2][0] == "cloud" and not r[2][2], str(r))
        check(tag + "OpenRouter badge not Off", badge(pg, "cloud") != "Off", badge(pg, "cloud"))
        move(pg, "cloud", "up")
        move(pg, "cloud", "up")
        check(tag + "OpenRouter first", [x[0] for x in rows(pg)] == ["cloud", "server", "local"], str(rows(pg)))
        line = pg.inner_text("#ai_engine_line")
        check(tag + "now-line starts with OpenRouter", line.startswith("OpenRouter ("), line)
        pg.screenshot(path=str(out / f"cleanup-order-moved-{scheme}.png"), full_page=True)

        # The Developer page carries no cleanup order.
        pg.evaluate("window.__goPage('settings-developer')")
        pg.wait_for_timeout(200)
        dev_text = pg.inner_text("body")
        check(tag + "Developer page has no cleanup order", "Cleanup order" not in dev_text or not pg.is_visible("#cleanup_order_group"))
        pg.close()
    check("no page errors", not errors, "; ".join(errors))
    b.close()

print("FAILED: " + ", ".join(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
