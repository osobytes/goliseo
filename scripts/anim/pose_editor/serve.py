#!/usr/bin/env python3
"""Serve the GOLISEO pose editor.

  python3 scripts/anim/pose_editor/serve.py            # http://127.0.0.1:8763

Serves this directory, maps /data/* to build/anim/editor/*, and accepts
POST /save to write an edited clip JSON back (with a .bak of the previous
version). Local tool only — never exposed beyond localhost.
"""
import json
import os
import shutil
from http.server import HTTPServer, SimpleHTTPRequestHandler

TOOL_DIR = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(TOOL_DIR, "..", "..", ".."))
# NOT under build/: concurrent tooling wipes build/, and this directory holds
# the user's saved pose edits
DATA_DIR = os.path.join(REPO, "scripts", "anim", "editor_data")
PORT = 8763


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=TOOL_DIR, **kw)

    def end_headers(self):
        # embedded preview panes load pages with an opaque origin; module
        # fetches enforce CORS, so allow all — this is a localhost-only tool
        self.send_header("Access-Control-Allow-Origin", "*")
        # never cache: a cached pre-CORS response revalidated via 304 keeps
        # its old headers and silently blocks module loading
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        # force full 200s — see Cache-Control note
        if "If-Modified-Since" in self.headers:
            del self.headers["If-Modified-Since"]
        self._get()

    def guess_type(self, path):
        t = super().guess_type(path)
        if t.startswith("text/") or t in ("application/javascript", "application/json"):
            return f"{t}; charset=utf-8"
        return t

    def _get(self):
        if self.path == "/clips":
            names = sorted(
                f[: -len(".clip.json")]
                for f in os.listdir(DATA_DIR)
                if f.endswith(".clip.json")
            )
            body = json.dumps({"clips": names}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path.startswith("/data/"):
            name = os.path.basename(self.path[len("/data/"):])
            full = os.path.join(DATA_DIR, name)
            if os.path.isfile(full):
                self.send_response(200)
                ctype = "model/gltf-binary" if name.endswith(".glb") else "application/json; charset=utf-8"
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(os.path.getsize(full)))
                self.end_headers()
                with open(full, "rb") as fh:
                    shutil.copyfileobj(fh, self.wfile)
                return
            self.send_error(404)
            return
        super().do_GET()

    def do_POST(self):
        if self.path != "/save":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", 0))
        payload = json.loads(self.rfile.read(length))
        name = os.path.basename(payload["file"])
        if not name.endswith(".clip.json"):
            self.send_error(400)
            return
        full = os.path.join(DATA_DIR, name)
        if os.path.exists(full):
            shutil.copy2(full, full + ".bak")
        with open(full, "w") as fh:
            json.dump(payload["data"], fh, indent=1)
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok": true}')
        print(f"saved {name}")


if __name__ == "__main__":
    print(f"GOLISEO pose editor:  http://127.0.0.1:{PORT}/?clip=kick_strike")
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
