#!/usr/bin/env python3
# V-10 host-side vsock wire capture (THROWAWAY SPIKE).
#
# Binds AF_PACKET/SOCK_RAW to the v10-vsockmon0 link and reads each vsockmon
# frame. The host vhost driver taps every guest TX packet via
# virtio_transport_deliver_tap_pkt() BEFORE the src_cid acceptance check
# (drivers/vhost/vsock.c), so this capture shows the *as-sent* src_cid --
# including a forged one -- and whether the host ever replies (op=CONNECT RST
# vs RESPONSE). That is the independent proof the forge actually crossed the
# virtqueue to the host, distinct from the host listener's "did it arrive".
#
# Each frame begins with struct af_vsockmon_hdr (uapi/linux/vsockmon.h):
#   __le64 src_cid; __le64 dst_cid; __le32 src_port; __le32 dst_port;
#   __le16 op; __le16 transport; __le16 len; __u8 reserved[2];  (32 bytes)
#
# Usage: capture.py <ifname> <seconds> <outpath>
import socket, struct, sys, time

IFNAME, SECONDS, OUT = sys.argv[1], int(sys.argv[2]), sys.argv[3]
OPS = {0: "UNKNOWN", 1: "CONNECT", 2: "DISCONNECT", 3: "CONTROL", 4: "PAYLOAD"}
ETH_P_ALL = 3

s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(ETH_P_ALL))
s.bind((IFNAME, 0))
s.settimeout(0.5)
end = time.time() + SECONDS
n = 0
with open(OUT, "w") as f:
    f.write(f"# vsockmon capture ifname={IFNAME} seconds={SECONDS}\n")
    f.flush()
    while time.time() < end:
        try:
            frame = s.recv(65535)
        except socket.timeout:
            continue
        if len(frame) < 32:
            continue
        src_cid, dst_cid, src_port, dst_port, op, transport, hlen = struct.unpack_from("<QQIIHHH", frame, 0)
        n += 1
        f.write("t=%0.3f op=%s src_cid=%d dst_cid=%d src_port=%d dst_port=%d transport=%d raw=%s\n" % (
            time.time(), OPS.get(op, op), src_cid, dst_cid, src_port, dst_port, transport,
            frame[:48].hex()))
        f.flush()
    f.write(f"# frames={n}\n")
print(f"[capture] {IFNAME}: {n} frames -> {OUT}", flush=True)
