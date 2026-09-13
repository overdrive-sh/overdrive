#!/usr/bin/env python3
"""Identity-unaware HTTP/1.1 workload used by the public-ingress journey."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:
        if self.path == "/abort":
            print("workload: abort-request-received", flush=True)
            self.connection.shutdown(2)
            self.connection.close()
            return
        chunks = (b"public-ingress-", b"gateway-", b"stream-ok\n")
        total = sum(map(len, chunks))
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(total))
        self.send_header("X-Workload", "api")
        self.end_headers()
        for chunk in chunks:
            self.wfile.write(chunk)
            self.wfile.flush()

    def log_message(self, format: str, *args: object) -> None:
        print(f"workload: {format % args}", flush=True)


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
