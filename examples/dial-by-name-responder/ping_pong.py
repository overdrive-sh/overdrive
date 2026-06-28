#!/usr/bin/env python3
"""dial-by-name-responder ping-pong demo program (S-DBN-PINGPONG / US-DBN-3).

The client program both `a.toml` and `b.toml` run, parameterised by args. It is
a REAL, checked-in, self-contained file living next to the example specs — run
it by hand outside the platform with plain Python, no build step:

    # terminal 1 — the "b" half
    python3 ping_pong.py --self-port 18972 --peer-name 127.0.0.1 --peer-port 18971
    # terminal 2 — the "a" half
    python3 ping_pong.py --self-port 18971 --peer-name 127.0.0.1 --peer-port 18972

(Outside the mesh, pass an IP / localhost as --peer-name. Inside the mesh the
spec passes the peer's MESH NAME — `b.svc.overdrive.local` / `a.svc.overdrive.local`
— which resolves via the in-agent DNS responder.)

Behaviour (mirrors the demo contract):

- Binds a TCP listener on `--self-port` (0.0.0.0). On each inbound connection it
  increments an inbound counter, stamps a fresh date, and replies
  `PONG count=<n> date=<iso8601>` — the "pong" the peer's dial reads (the counter
  + date an operator watches advance).
- On a ~10s loop it resolves `--peer-name` via `socket.getaddrinfo`
  (getaddrinfo — the stub resolver / production resolv.conf path, NOT `dig`) and
  dials it over an ORDINARY plaintext TcpStream. The workload is identity-unaware
  and presents NO TLS / SNI; the platform's agent transparently originates mTLS
  on the inter-agent leg (CLAUDE.md "East-west mTLS tests"). On each successful
  exchange it bumps a round-trip counter and prints the peer's reply.

stdlib only; no third-party deps; runs on the `/usr/bin/python3` already present
in the dev Lima VM.
"""

import argparse
import datetime
import socket
import sys
import threading
import time

CADENCE_SECS = 10
DIAL_TIMEOUT_SECS = 5


def _iso_now() -> str:
    return datetime.datetime.now().isoformat(timespec="seconds")


def _serve(self_port: int) -> None:
    """Inbound: accept connections forever; reply a counted, dated PONG."""
    inbound = 0
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("0.0.0.0", self_port))
    srv.listen(8)
    print(f"[ping-pong {self_port}] listening", flush=True)
    while True:
        try:
            conn, _ = srv.accept()
        except OSError:
            continue
        with conn:
            try:
                _ = conn.recv(256)
                inbound += 1
                pong = f"PONG count={inbound} date={_iso_now()}\n"
                conn.sendall(pong.encode())
            except OSError:
                pass


def _dial_loop(self_port: int, peer_name: str, peer_port: int) -> None:
    """Outbound: every ~10s resolve the peer BY NAME (getaddrinfo) + dial plaintext."""
    outbound = 0
    while True:
        try:
            # getaddrinfo — the real stub-resolver call (production resolv.conf
            # path), NOT `dig`. Inside the mesh `peer_name` is the mesh name and
            # resolves to the peer's stable frontend F.
            infos = socket.getaddrinfo(
                peer_name, peer_port, socket.AF_INET, socket.SOCK_STREAM
            )
            addr = infos[0][4]
            # Plaintext dial — no TLS, no SNI. The agent originates mTLS on the
            # inter-agent leg-B <-> leg-C wire; this workload never sees it.
            with socket.create_connection(addr, timeout=DIAL_TIMEOUT_SECS) as tcp:
                tcp.sendall(f"PING from {self_port}\n".encode())
                reply = tcp.recv(256).decode(errors="replace").strip()
                if reply:
                    outbound += 1
                    print(
                        f"[ping-pong {self_port}] outbound#{outbound} -> {peer_name} got: {reply}",
                        flush=True,
                    )
        except OSError as err:
            print(
                f"[ping-pong {self_port}] dial {peer_name}:{peer_port} failed: {err}",
                file=sys.stderr,
                flush=True,
            )
        time.sleep(CADENCE_SECS)


def main() -> None:
    parser = argparse.ArgumentParser(description="dial-by-name ping-pong demo")
    parser.add_argument("--self-port", type=int, required=True, help="TCP listener port")
    parser.add_argument("--peer-name", required=True, help="peer mesh name (or IP) to dial")
    parser.add_argument("--peer-port", type=int, required=True, help="peer's TCP port")
    args = parser.parse_args()

    threading.Thread(
        target=_serve, args=(args.self_port,), daemon=True
    ).start()
    _dial_loop(args.self_port, args.peer_name, args.peer_port)


if __name__ == "__main__":
    main()
