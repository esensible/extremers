#!/usr/bin/env python3
"""Dev server for the studio: static files, no caching, correct MIME types.

Origin: dingo-cli/web/serve.py (verbatim but for the port and two types).

Run inside the dev container (never on the host):
    python3 studio/serve.py            # serves studio/ on 0.0.0.0:5190
The container must publish 5190 to the host's 127.0.0.1: Chrome treats
http://localhost:5190 as a secure context (Web Bluetooth, service workers).
Keep the port: IndexedDB projects and Chrome's Bluetooth permission are
tied to the origin.
"""
import http.server, os, sys

class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map,
                      ".wasm": "application/wasm", ".js": "text/javascript", ".mjs": "text/javascript",
                      ".jsx": "text/plain; charset=utf-8", ".expr": "text/plain; charset=utf-8",
                      ".json": "application/json", ".md": "text/markdown; charset=utf-8"}
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

if __name__ == "__main__":
    os.chdir(os.path.dirname(os.path.abspath(__file__)))
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 5190
    http.server.ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()
