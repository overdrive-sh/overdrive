#!/usr/bin/env bash
# V-16 control: prove the receiver configuration actually VALIDATES UDP checksums (a bad checksum must be dropped,
# csum 0 and a good checksum accepted), so the byte-exact results above are not an artifact of an unchecked receiver.
set -u
cleanup() { ip link del v16-a 2>/dev/null; ip netns del v16-rx 2>/dev/null; }
trap cleanup EXIT; cleanup
ip netns add v16-rx; ip link add v16-a type veth peer name v16-b; ip link set v16-b netns v16-rx
ip addr add 10.216.16.1/30 dev v16-a; ip link set v16-a up
ip -n v16-rx addr add 10.216.16.2/30 dev v16-b; ip -n v16-rx link set v16-b up; ip -n v16-rx link set lo up
ip netns exec v16-rx ethtool -K v16-b rx off >/dev/null; ethtool -K v16-a tx off >/dev/null
echo "uname -r: $(uname -r)"; echo "v16-b $(ip netns exec v16-rx ethtool -k v16-b | grep '^rx-checksumming')"
ip netns exec v16-rx python3 - <<'PY' &
import socket, time
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("10.216.16.2", 5555)); s.settimeout(3)
got = []
t = time.time()
while time.time() - t < 3.5:
    try: d, _ = s.recvfrom(100); got.append(d)
    except socket.timeout: break
print("CONTROL receiver got:", got)
PY
sleep 1
python3 - <<'PY'
import socket, struct
def csum(b):
    if len(b) % 2: b += b"\0"
    t = sum(struct.unpack(">%dH" % (len(b)//2), b))
    while t >> 16: t = (t & 0xffff) + (t >> 16)
    return (~t) & 0xffff
src, dst = socket.inet_aton("10.216.16.1"), socket.inet_aton("10.216.16.2")
def pkt(payload, mode):
    udp = struct.pack(">HHHH", 4444, 5555, 8 + len(payload), 0) + payload
    if mode == "good": c = csum(src + dst + b"\0\x11" + struct.pack(">H", len(udp)) + udp)
    elif mode == "bad": c = 0xdead
    else: c = 0
    udp = udp[:6] + struct.pack(">H", c) + udp[8:]
    ip = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), 1, 0, 64, 17, 0, src, dst)
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return ip + udp
r = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_RAW)
for m in ("bad", "good", "zero"):
    r.sendto(pkt(b"csum-" + m.encode(), m), ("10.216.16.2", 0))
print("CONTROL sent: bad(0xdead), good, zero")
PY
wait
echo "-- netstat UDP InCsumErrors in receiver ns:"; ip netns exec v16-rx nstat -az UdpInCsumErrors 2>&1 | tail -2
