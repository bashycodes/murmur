"""Serve ui/dist with mock-tauri.js injected into settings.html.

Probe for checking the settings window in a plain browser (no Tauri):
    python3 tools/probes/settings_mock/serve.py [port]
then open http://127.0.0.1:<port>/settings.html.
"""
import http.server
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
DIST = HERE.parents[2] / "ui" / "dist"


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=str(DIST), **k)

    def do_GET(self):
        if self.path.startswith("/mock-tauri.js"):
            body = (HERE / "mock-tauri.js").read_bytes()
            ctype = "text/javascript"
        elif self.path.startswith("/settings.html"):
            html = (DIST / "settings.html").read_text()
            body = html.replace(
                '<script src="settings.js">',
                '<script src="mock-tauri.js"></script><script src="settings.js">',
            ).encode()
            ctype = "text/html"
        else:
            return super().do_GET()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
