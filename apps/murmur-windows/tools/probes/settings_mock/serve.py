"""Serve ui/dist with mock-tauri.js injected into settings.html (and
mock-scratchpad.js into scratchpad.html, mock-overlay.js into overlay.html).

Probe for checking the settings window in a plain browser (no Tauri):
    python3 tools/probes/settings_mock/serve.py [port]
then open http://127.0.0.1:<port>/settings.html (or /scratchpad.html).

The same tree is also served under /notetaker-on/ with the UI half of the
Notetaker switch (D51, `NOTETAKER_ENABLED` in ui/src/pages/notetaker.ts)
flipped to true, so the Meetings probes (check_meetings.py,
check_meeting_order.py) run against the committed dist while the release
keeps the Notetaker hidden. Open /notetaker-on/settings.html for that view.
"""
import http.server
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
DIST = HERE.parents[2] / "ui" / "dist"
NOTETAKER_ON = "/notetaker-on"
NOTETAKER_OFF_LINE = b"export const NOTETAKER_ENABLED = false;"
NOTETAKER_ON_LINE = b"export const NOTETAKER_ENABLED = true;"


def notetaker_on_js():
    """dist/pages/notetaker.js with the switch on, or the reason it can't be
    flipped. Checked once at startup, so the Meetings probes never silently
    test the hidden UI."""
    js = (DIST / "pages" / "notetaker.js").read_bytes()
    if NOTETAKER_ON_LINE in js:
        return js
    if js.count(NOTETAKER_OFF_LINE) != 1:
        raise SystemExit(
            "serve.py: `export const NOTETAKER_ENABLED = false;` not found exactly once in "
            "ui/dist/pages/notetaker.js, so /notetaker-on/ can't switch the Notetaker on. "
            "Update NOTETAKER_OFF_LINE in serve.py."
        )
    return js.replace(NOTETAKER_OFF_LINE, NOTETAKER_ON_LINE)


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=str(DIST), **k)

    def do_GET(self):
        if self.path == NOTETAKER_ON or self.path.startswith(NOTETAKER_ON + "/"):
            self.path = self.path[len(NOTETAKER_ON):] or "/"
            if self.path.split("?")[0] == "/pages/notetaker.js":
                return self.send_body(self.server.notetaker_on_js, "text/javascript")
        if self.path.startswith("/mock-tauri.js") or self.path.startswith(
            "/mock-scratchpad.js"
        ) or self.path.startswith("/mock-overlay.js"):
            body = (HERE / self.path[1:].split("?")[0]).read_bytes()
            ctype = "text/javascript"
        elif self.path.startswith("/scratchpad.html"):
            html = (DIST / "scratchpad.html").read_text()
            body = html.replace(
                '<script src="scratchpad.js">',
                '<script src="mock-scratchpad.js"></script><script src="scratchpad.js">',
            ).encode()
            ctype = "text/html"
        elif self.path.startswith("/overlay.html"):
            html = (DIST / "overlay.html").read_text()
            body = html.replace(
                '<script src="overlay.js">',
                '<script src="mock-overlay.js"></script><script src="overlay.js">',
            ).encode()
            ctype = "text/html"
        elif self.path.startswith("/settings.html"):
            html = (DIST / "settings.html").read_text()
            body = html.replace(
                '<script type="module" src="settings.js">',
                '<script src="mock-tauri.js"></script><script type="module" src="settings.js">',
            ).encode()
            ctype = "text/html"
        else:
            return super().do_GET()
        self.send_body(body, ctype)

    def send_body(self, body, ctype):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    notetaker_js = notetaker_on_js()  # exits non-zero with the reason if the line moved
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.notetaker_on_js = notetaker_js
    server.serve_forever()
