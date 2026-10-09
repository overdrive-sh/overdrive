#!/usr/bin/env python3
"""V-13 edge case: leg-F's remote socket R is a kTLS socket in the design (mTLS port). This holder
connects to the remote peer, optionally sets SO_LINGER{1,0}, installs the kernel TLS ULP with TX
(AES-GCM-128, fixed keys: confidentiality is irrelevant here) and streams records continuously
until it is SIGKILLed. Usage: ktls_holder.py <ip:port> <linger yes|no> <tls yes|no>"""
import socket, struct, sys, time
ip, port = sys.argv[1].split(":"); linger = sys.argv[2] == "yes"; tls = sys.argv[3] == "yes"
SOL_TLS, TLS_TX, TLS_RX, TCP_ULP = 282, 1, 2, 31
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
if linger:
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
s.connect((ip, int(port)))
if tls:
    s.setsockopt(socket.IPPROTO_TCP, TCP_ULP, b"tls")
    # struct tls12_crypto_info_aes_gcm_128: version, cipher_type, iv[8], key[16], salt[4], rec_seq[8]
    info = struct.pack("HH8s16s4s8s", 0x0303, 51, b"\x01" * 8, b"\x02" * 16, b"\x03" * 4, b"\x00" * 8)
    s.setsockopt(SOL_TLS, TLS_TX, info)
print("HOLDER_READY linger=%s tls=%s" % (linger, tls), flush=True)
buf = b"x" * 4096
s.setblocking(False)
while True:
    try:
        s.send(buf)
    except BlockingIOError:
        time.sleep(0.0005)
    try:
        s.recv(65536)
    except BlockingIOError:
        pass
    time.sleep(0.0002)
