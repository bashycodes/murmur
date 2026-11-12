#!/usr/bin/env python3
"""D84 live-test backend: an OpenAI-compatible `/v1/audio/transcriptions`
server for a LiteLLM proxy to route an `audio_transcription` model to.

Dated probe (2026-11-12, `feat/litellm-stt`); not a product. With
`--whisper-cli` and `--model` it transcribes for real with whisper.cpp
(the uploaded file as it came: FLAC or WAV); without them it answers a
fixed text. It logs the multipart field names and sizes, never the audio.

    python3 whisper_backend.py --port 8093 \
        --whisper-cli <whisper.cpp>/build/bin/whisper-cli \
        --model <models>/ggml-tiny-q5_1.bin

LiteLLM config entry:

    - model_name: local-whisper
      litellm_params:
        model: openai/whisper-1
        api_base: http://127.0.0.1:8093/v1
        api_key: none
      model_info:
        mode: audio_transcription
"""

import argparse
import email.parser
import email.policy
import json
import os
import subprocess
import sys
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ARGS = None


def parse_multipart(ctype: str, body: bytes) -> dict:
    """Field name -> (filename, bytes)."""
    msg = email.parser.BytesParser(policy=email.policy.HTTP).parsebytes(
        b"Content-Type: " + ctype.encode() + b"\r\n\r\n" + body
    )
    out = {}
    for part in msg.iter_parts():
        name = part.get_param("name", header="content-disposition")
        out[name] = (part.get_filename(), part.get_payload(decode=True) or b"")
    return out


def transcribe(filename: str, data: bytes, language: str) -> str:
    if not ARGS.whisper_cli:
        return ARGS.text
    ext = os.path.splitext(filename or "audio.wav")[1] or ".wav"
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "audio" + ext)
        with open(p, "wb") as f:
            f.write(data)
        cmd = [ARGS.whisper_cli, "-m", ARGS.model, "-f", p, "-nt", "-np",
               "-l", language or "en", "-t", "2"]
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        if r.returncode != 0:
            raise RuntimeError(r.stderr[-400:])
        return " ".join(r.stdout.split())


class H(BaseHTTPRequestHandler):
    def _json(self, code: int, obj) -> None:
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path.rstrip("/") in ("/v1/models", "/models"):
            return self._json(200, {"data": [{"id": "whisper-1"}]})
        self._json(404, {"error": {"message": "not found"}})

    def do_POST(self):
        if not self.path.rstrip("/").endswith("/audio/transcriptions"):
            return self._json(404, {"error": {"message": "not found"}})
        n = int(self.headers.get("content-length") or 0)
        f = parse_multipart(self.headers.get("content-type", ""), self.rfile.read(n))
        shape = {k: len(v[1]) for k, v in f.items()}
        sys.stderr.write(f"transcriptions: fields {shape} file {f.get('file', (None,))[0]}\n")
        if "file" not in f:
            return self._json(400, {"error": {"message": "no file"}})
        lang = (f.get("language") or (None, b""))[1].decode()
        try:
            text = transcribe(f["file"][0], f["file"][1], lang)
        except Exception as e:  # noqa: BLE001 - a probe: report and go on
            return self._json(500, {"error": {"message": str(e)}})
        self._json(200, {"text": text})

    def log_message(self, *a):
        pass


def main() -> None:
    global ARGS
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8093)
    ap.add_argument("--whisper-cli")
    ap.add_argument("--model")
    ap.add_argument("--text", default="and so my fellow americans")
    ARGS = ap.parse_args()
    ThreadingHTTPServer(("127.0.0.1", ARGS.port), H).serve_forever()


if __name__ == "__main__":
    main()
