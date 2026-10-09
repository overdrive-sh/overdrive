#!/usr/bin/env python3
"""V-21 peer-netns services (throwaway). Stand-ins for off-host destinations:
instance metadata 169.254.169.254:80, a public host 198.51.100.7:80, the host's neighbour 10.77.21.2:22022.
Each accept / datagram appends one line to the log file so the probe can count hits."""
import socket, sys, threading
log = open(sys.argv[1], "a", buffering=1)
lk = threading.Lock()
def note(s):
    with lk: log.write(s + "\n")
def tcp(addr, port):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((addr, port)); s.listen(128)
    while True:
        c, _ = s.accept()
        try:
            c.settimeout(0.5); c.recv(16); note(f"tcp {addr}:{port}"); c.sendall(b"R")
        except OSError: note(f"tcp {addr}:{port} (no data)")
        c.close()
def udp(addr, port):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind((addr, port))
    while True:
        d, a = s.recvfrom(70000); note(f"udp {addr}:{port}"); s.sendto(b"R", a)
for a, p in [("10.77.21.2", 22022), ("198.51.100.7", 80), ("169.254.169.254", 80)]:
    for f in (tcp, udp): threading.Thread(target=f, args=(a, p), daemon=True).start()
note("ready")
threading.Event().wait()
