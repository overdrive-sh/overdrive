#!/usr/bin/env python3
"""Spike A host-side listener (GH #303 in-guest-kernel-mtls).

Two one-shot listeners, each records what it receives and replies with a
byte-distinct RESPONSE:

  vsock leg: AF_UNIX listener at "<vsock-base>_<port>" -- the path Cloud
             Hypervisor's muxer connects to for a guest-initiated vsock
             connect to (CID 2, port) (virtio-devices/src/vsock/unix/muxer.rs
             v53.0:716, format!("{}_{}", host_sock_path, dst_port)).
  net leg:   AF_INET listener on the tap address, bound with IP_FREEBIND so
             it can bind before Cloud Hypervisor creates the tap.

Prints one line per event with seconds since the launcher's T0 (argv) so the
run log carries a host-observed boot-to-first-vsock time.
"""
import os
import select
import socket
import sys
import time

VSOCK_REQ = b"IGKM-A-VSOCK-REQUEST guest->host\n"
VSOCK_RESP = b"IGKM-A-VSOCK-RESPONSE host->guest\n"
NET_REQ = b"IGKM-A-NET-REQUEST guest->host\n"
NET_RESP = b"IGKM-A-NET-RESPONSE host->guest\n"
IP_FREEBIND = 15


def main() -> int:
    uds_path, net_addr, net_port, t0, deadline_s = (
        sys.argv[1], sys.argv[2], int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5]))

    def t() -> str:
        return f"t+{time.time() - t0:.6f}s"

    def log(msg: str) -> None:
        print(f"HOST: {msg} {t()}", flush=True)

    if os.path.exists(uds_path):
        log(f"refusing: {uds_path} already exists")
        return 2
    vs = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    vs.bind(uds_path)
    vs.listen(1)
    log(f"vsock-leg listening on unix {uds_path}")

    ns = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    ns.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    ns.setsockopt(socket.IPPROTO_IP, IP_FREEBIND, 1)
    ns.bind((net_addr, net_port))
    ns.listen(1)
    log(f"net-leg listening on tcp {net_addr}:{net_port} (IP_FREEBIND)")

    pending = {vs: ("vsock", VSOCK_REQ, VSOCK_RESP), ns: ("net", NET_REQ, NET_RESP)}
    results = {}
    end = time.time() + deadline_s
    while pending and time.time() < end:
        ready, _, _ = select.select(list(pending), [], [], 0.2)
        for lsock in ready:
            leg, expect, resp = pending.pop(lsock)
            conn, peer = lsock.accept()
            log(f"{leg}-leg accepted connection peer={peer!r}")
            conn.settimeout(10)
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(256)
                if not chunk:
                    break
                data += chunk
            log(f"{leg}-leg received {len(data)} bytes: {data!r}")
            ok = data == expect
            if ok:
                conn.sendall(resp)
                log(f"{leg}-leg replied {len(resp)} bytes: {resp!r}")
            else:
                log(f"{leg}-leg REQUEST MISMATCH (expected {expect!r}); not replying")
            results[leg] = ok
            conn.close()
            lsock.close()
    for leg, _, _ in pending.values():
        log(f"{leg}-leg NO CONNECTION before deadline")
        results[leg] = False
    for lsock in pending:
        lsock.close()
    if os.path.exists(uds_path):
        os.unlink(uds_path)
    for leg in ("vsock", "net"):
        log(f"VERDICT {leg}={'PASS' if results.get(leg) else 'FAIL'}")
    return 0 if all(results.get(leg) for leg in ("vsock", "net")) else 1


if __name__ == "__main__":
    sys.exit(main())
